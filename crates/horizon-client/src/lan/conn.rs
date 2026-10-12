//! One accepted connection: handshake, status ping, offline-mode login and the
//! configuration phase (ServerStatusPacketListenerImpl,
//! ServerLoginPacketListenerImpl, ServerConfigurationPacketListenerImpl). A
//! player that completes configuration continues in the play state: the
//! reader forwards decoded packets to the game thread while a writer thread
//! frames, compresses and encrypts what the game queues, and answers the
//! keep-alive schedule on its own so a long frame never times a client out.
use super::{
    Joined, NetEvent, Out, Shared,
    data::{CORE_PACK, SyncData},
};
use horizon_protocol::{
    PROTOCOL_VERSION,
    connection::{ConnReader, ConnWriter, Connection},
    crypto::offline_uuid,
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation, KnownPack},
        handshake, login, play, status,
    },
    text::translatable,
};
use std::{
    net::TcpStream,
    sync::{
        Arc,
        atomic::{AtomicI32, AtomicI64, AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    time::{Duration, Instant},
};

/// ServerCommonPacketListenerImpl.KEEPALIVE_TIME and the disconnect timeout.
const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(15);
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(30);
/// Packets queued for one client beyond this many bytes disconnect it.
const MAX_PENDING: usize = 96 << 20;
/// Vanilla's server-side disconnect during login is a JSON component.
fn login_disconnect(c: &mut Connection, reason: &horizon_world::nbt::Tag) {
    let json = component_json(reason);
    let _ =
        c.send(&login::Clientbound::Disconnect(login::Disconnect { reason_json: json }).encode());
}

/// A component as ComponentSerialization.CODEC writes it with JsonOps (and
/// Gson without HTML escaping): plain text collapses to a string, the
/// content fields come first, then `extra`, then the style.
pub fn component_json(tag: &horizon_world::nbt::Tag) -> String {
    component_value(tag).compact()
}
fn component_value(tag: &horizon_world::nbt::Tag) -> crate::server::json::Json {
    use crate::server::json::Json;
    use horizon_world::nbt::Tag;
    const ORDER: &[&str] = &[
        "type", "text", "translate", "fallback", "with", "score", "selector", "separator",
        "keybind", "nbt", "interpret", "block", "entity", "storage", "source", "object",
        "sprite", "atlas", "player", "hat", "extra", "color", "shadow_color", "bold", "italic",
        "underlined", "strikethrough", "obfuscated", "click_event", "hover_event", "insertion",
        "font",
    ];
    match tag {
        Tag::String(s) => Json::String(s.to_string_lossy()),
        Tag::List(items) => Json::Array(items.iter().map(component_value).collect()),
        Tag::Compound(map) => {
            if map.len() == 1
                && let Some(Tag::String(t)) = map.get(&horizon_world::nbt::JavaString::from("text"))
            {
                return Json::String(t.to_string_lossy());
            }
            let mut keys: Vec<String> = map.keys().map(|k| k.to_string_lossy()).collect();
            keys.sort_by_key(|k| ORDER.iter().position(|o| o == k).unwrap_or(ORDER.len()));
            Json::Object(
                keys.into_iter()
                    .map(|k| {
                        let v = &map[&horizon_world::nbt::JavaString::from(k.as_str())];
                        let value = match v {
                            Tag::Byte(b) => Json::Bool(*b != 0),
                            Tag::Int(i) => Json::Number(i.to_string()),
                            other => component_value(other),
                        };
                        (k, value)
                    })
                    .collect(),
            )
        }
        Tag::Byte(b) => Json::Bool(*b != 0),
        Tag::Int(i) => Json::Number(i.to_string()),
        Tag::Long(i) => Json::Number(i.to_string()),
        _ => Json::Null,
    }
}

/// StringUtil.isValidPlayerName.
pub fn valid_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.chars().all(|c| c as u32 > 32 && (c as u32) < 127)
}

/// The sending half given to the game thread.
#[derive(Clone)]
pub struct PlayerSender {
    pub tx: Sender<Out>,
    pub pending: Arc<AtomicUsize>,
}
impl PlayerSender {
    pub fn send(&self, body: Vec<u8>) {
        self.pending.fetch_add(body.len(), Ordering::Relaxed);
        let _ = self.tx.send(Out::Packet(body));
    }
}

/// LegacyQueryHandler: the pre-1.7 server list ping. What the first read
/// holds decides the reply (1.3 and older: no more bytes; 1.4 and 1.5: 0x01;
/// 1.6: 0x01 then the MC|PingHost payload); anything else gets no reply.
fn serve_legacy(mut stream: TcpStream, shared: &Shared) -> Result<(), String> {
    use std::io::{Read, Write};
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut data = Vec::new();
    let mut chunk = [0u8; 1024];
    // The bytes of the first read: what arrived together.
    if let Ok(n) = stream.read(&mut chunk) {
        data.extend_from_slice(&chunk[..n]);
    }
    let in_string = |d: &[u8], at: &mut usize| -> Option<String> {
        let len = usize::from(u16::from_be_bytes([*d.get(*at)?, *d.get(*at + 1)?]));
        let bytes = d.get(*at + 2..*at + 2 + len * 2)?;
        *at += 2 + len * 2;
        Some(String::from_utf16_lossy(
            &bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>(),
        ))
    };
    let motd = shared.motd();
    let (online, max) = (shared.online(), shared.max_players());
    let body = if data.len() == 1 {
        format!("{motd}\u{a7}{online}\u{a7}{max}")
    } else {
        if data[1] != 1 {
            return Ok(());
        }
        if data.len() > 2 {
            // readCustomPayloadPacket.
            let mut at = 3;
            let valid = data[2] == 0xFA
                && in_string(&data, &mut at).as_deref() == Some("MC|PingHost")
                && data
                    .get(at..at + 2)
                    .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
                    == Some(data.len().saturating_sub(at + 2))
                && {
                    at += 2;
                    data.get(at).is_some_and(|v| *v >= 73)
                }
                && {
                    at += 1;
                    in_string(&data, &mut at).is_some()
                }
                && data
                    .get(at..at + 4)
                    .is_some_and(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]) <= 65535);
            if !valid {
                return Ok(());
            }
        }
        format!("\u{a7}1\0127\026.3\0{motd}\0{online}\0{max}")
    };
    let units: Vec<u16> = body.encode_utf16().collect();
    let mut out = vec![0xFF];
    out.extend_from_slice(&(units.len() as u16).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
    stream.write_all(&out).map_err(|e| e.to_string())?;
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

pub fn serve(
    stream: TcpStream,
    address: std::net::SocketAddr,
    shared: Arc<Shared>,
) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    // LegacyQueryHandler, ahead of the frame decoder while status is on.
    let mut first = [0u8; 1];
    if shared.config.enable_status && stream.peek(&mut first).is_ok_and(|n| n == 1) && first[0] == 0xFE {
        return serve_legacy(stream, &shared);
    }
    let mut c = Connection::from_stream(stream).map_err(e)?;
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    let intention = match handshake::Serverbound::decode(id, &payload).map_err(e)? {
        handshake::Serverbound::Intention(i) => i,
        _ => return Err("expected a handshake".into()),
    };
    match intention.intent {
        // ServerHandshakePacketListenerImpl: a server that does not reply
        // to status requests closes the connection.
        handshake::STATUS if !shared.config.enable_status => Ok(()),
        handshake::STATUS => serve_status(&mut c, &shared),
        handshake::TRANSFER if !shared.config.accepts_transfers => {
            login_disconnect(
                &mut c,
                &translatable("multiplayer.disconnect.transfers_disabled", &[]),
            );
            Ok(())
        }
        handshake::LOGIN | handshake::TRANSFER => {
            if intention.protocol != PROTOCOL_VERSION {
                let key = if intention.protocol < 754 {
                    "multiplayer.disconnect.outdated_client"
                } else {
                    "multiplayer.disconnect.incompatible"
                };
                let json = format!(
                    "{{\"translate\":\"{key}\",\"with\":[\"{}\"]}}",
                    horizon_protocol::GAME_VERSION
                );
                let _ = c.send(
                    &login::Clientbound::Disconnect(login::Disconnect { reason_json: json })
                        .encode(),
                );
                return Ok(());
            }
            serve_login(c, address, &shared)
        }
        other => Err(format!("unknown intent {other}")),
    }
}

fn serve_status(c: &mut Connection, shared: &Shared) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let mut requested = false;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match status::Serverbound::decode(id, &payload).map_err(e)? {
            // ServerStatusPacketListenerImpl: a second request disconnects.
            status::Serverbound::StatusRequest(_) if requested => return Ok(()),
            status::Serverbound::StatusRequest(_) => {
                requested = true;
                let json = shared.status_json();
                c.send(
                    &status::Clientbound::StatusResponse(status::StatusResponse { json }).encode(),
                )
                .map_err(e)?;
            }
            status::Serverbound::PingRequest(p) => {
                c.send(&status::Clientbound::PongResponse(p).encode())
                    .map_err(e)?;
                return Ok(());
            }
            status::Serverbound::Unknown(..) => return Ok(()),
        }
    }
}

fn serve_login(
    mut c: Connection,
    address: std::net::SocketAddr,
    shared: &Arc<Shared>,
) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let (id, payload) = c.recv().map_err(e)?;
    let hello = match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::Hello(h) => h,
        _ => return Err("expected a login hello".into()),
    };
    if !valid_name(&hello.name) {
        login_disconnect(
            &mut c,
            &translatable("multiplayer.disconnect.invalid_name", &[]),
        );
        return Ok(());
    }
    // Offline mode: the profile is derived from the name (PlayerList.canPlayerLogin
    // then rejects duplicates and a full server).
    let uuid = offline_uuid(&hello.name);
    if let Some(reason) = shared.try_admit(address, uuid, &hello.name) {
        login_disconnect(&mut c, &reason);
        return Ok(());
    }
    let result = login_and_configure(&mut c, shared, uuid, &hello.name);
    match result {
        Ok(Some(parts)) => {
            play(c, shared, parts, address);
            Ok(())
        }
        Ok(None) => {
            shared.release(uuid);
            Ok(())
        }
        Err(err) => {
            shared.release(uuid);
            Err(err)
        }
    }
}

struct Parts {
    uuid: u128,
    name: String,
    info: ClientInformation,
    brand: String,
}

fn login_and_configure(
    c: &mut Connection,
    shared: &Arc<Shared>,
    uuid: u128,
    name: &str,
) -> Result<Option<Parts>, String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let threshold = shared.config.compression_threshold;
    if threshold >= 0 {
        c.send(&login::Clientbound::Compression(login::Compression { threshold }).encode())
            .map_err(e)?;
        c.set_compression(threshold);
    }
    c.send(
        &login::Clientbound::LoginFinished(login::LoginFinished {
            profile: login::GameProfile {
                uuid,
                name: name.to_owned(),
                properties: Vec::new(),
            },
            session: uuid,
        })
        .encode(),
    )
    .map_err(e)?;
    // The client may send its settings before acknowledging the login.
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match login::Serverbound::decode(id, &payload).map_err(e)? {
            login::Serverbound::LoginAcknowledged(_) => break,
            login::Serverbound::CustomQueryAnswer(_) | login::Serverbound::CookieResponse(_) => {}
            _ => return Err("unexpected packet during login".into()),
        }
    }
    // Configuration.
    let sync: &SyncData = &shared.sync;
    let send = |c: &mut Connection, p: cfg::Clientbound| c.send(&p.encode()).map_err(e);
    send(
        c,
        cfg::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: {
                let mut w = horizon_protocol::buf::Writer::new();
                // MinecraftServer.getServerModName.
                w.string("vanilla");
                w.data
            },
        }),
    )?;
    send(
        c,
        cfg::Clientbound::UpdateEnabledFeatures(cfg::UpdateEnabledFeatures {
            features: sync.features.clone(),
        }),
    )?;
    let core = KnownPack {
        namespace: CORE_PACK.0.into(),
        id: CORE_PACK.1.into(),
        version: CORE_PACK.2.into(),
    };
    send(
        c,
        cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks {
            packs: vec![core.clone()],
        }),
    )?;
    let mut info = ClientInformation::new(10);
    let mut brand = String::from("vanilla");
    let known_pack = loop {
        let (id, payload) = c.recv().map_err(e)?;
        match cfg::Serverbound::decode(id, &payload).map_err(e)? {
            cfg::Serverbound::ClientInformation(i) => info = i,
            cfg::Serverbound::CustomPayload(p) if p.channel == "minecraft:brand" => {
                brand = horizon_protocol::buf::Reader::new(&p.data)
                    .string(32767)
                    .unwrap_or(brand);
            }
            cfg::Serverbound::SelectKnownPacks(p) => break p.packs == vec![core.clone()],
            _ => {}
        }
    };
    for packet in sync.registry_packets(known_pack) {
        send(c, cfg::Clientbound::RegistryData(packet))?;
    }
    send(c, cfg::Clientbound::UpdateTags(sync.tags.clone()))?;
    send(c, cfg::Clientbound::FinishConfiguration(Empty))?;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match cfg::Serverbound::decode(id, &payload).map_err(e)? {
            cfg::Serverbound::FinishConfiguration(_) => break,
            cfg::Serverbound::ClientInformation(i) => info = i,
            _ => {}
        }
    }
    Ok(Some(Parts {
        uuid,
        name: name.to_owned(),
        info,
        brand,
    }))
}

fn play(c: Connection, shared: &Arc<Shared>, parts: Parts, address: std::net::SocketAddr) {
    let conn_id = shared.next_id();
    let events = shared.events.clone();
    let _ = c.reader.set_read_timeout(Some(KEEP_ALIVE_TIMEOUT));
    let (mut reader, writer) = c.split();
    let (tx, rx) = channel();
    let pending = Arc::new(AtomicUsize::new(0));
    let latency = Arc::new(AtomicI32::new(0));
    let alive = Arc::new(AtomicI64::new(-1));
    let sender = PlayerSender {
        tx: tx.clone(),
        pending: pending.clone(),
    };
    let writer_thread = {
        let (alive, latency, pending) = (alive.clone(), latency.clone(), pending.clone());
        std::thread::Builder::new()
            .name(format!("horizon-lan-out-{conn_id}"))
            .spawn(move || write_loop(writer, rx, alive, latency, pending))
            .ok()
    };
    let joined = Joined {
        conn: conn_id,
        uuid: parts.uuid,
        name: parts.name,
        info: parts.info,
        brand: parts.brand,
        sender,
        latency,
        address,
    };
    let reason = if events.send(NetEvent::Joined(Box::new(joined))).is_ok() {
        read_loop(&mut reader, conn_id, &events, &alive)
    } else {
        "server closed".to_owned()
    };
    let _ = tx.send(Out::Close);
    let _ = events.send(NetEvent::Left(conn_id, reason));
    if let Some(t) = writer_thread {
        let _ = t.join();
    }
    shared.release(parts.uuid);
}

fn read_loop(
    reader: &mut ConnReader,
    conn: u32,
    events: &Sender<NetEvent>,
    alive: &AtomicI64,
) -> String {
    loop {
        let (id, payload) = match reader.recv() {
            Ok(p) => p,
            // The client closed the connection: Connection.channelInactive
            // without details, multiplayer.disconnect.generic.
            Err(e)
                if e.0 == "Connection closed by the server"
                    || e.0.contains("reset by peer")
                    || e.0.contains("aborted") =>
            {
                return "Disconnected".to_owned();
            }
            Err(e) => return e.to_string(),
        };
        match play::Serverbound::decode(id, &payload) {
            Ok(play::Serverbound::KeepAlive(k)) => alive.store(k.id, Ordering::Relaxed),
            Ok(p) => {
                if events.send(NetEvent::Packet(conn, Box::new(p))).is_err() {
                    return "server closed".to_owned();
                }
            }
            // A malformed packet disconnects like vanilla's decoder exception.
            Err(e) => return format!("Invalid packet {id}: {e}"),
        }
    }
}

fn write_loop(
    mut writer: ConnWriter,
    rx: Receiver<Out>,
    alive: Arc<AtomicI64>,
    latency: Arc<AtomicI32>,
    pending: Arc<AtomicUsize>,
) {
    let start = Instant::now();
    let mut sent_at: Option<(i64, Instant)> = None;
    let mut next = Instant::now() + KEEP_ALIVE_EVERY;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Out::Packet(body)) => {
                pending.fetch_sub(body.len(), Ordering::Relaxed);
                if writer.send(&body).is_err() {
                    break;
                }
                if pending.load(Ordering::Relaxed) > MAX_PENDING {
                    break;
                }
            }
            Ok(Out::Disconnect(reason)) => {
                let _ = writer
                    .send(&play::Clientbound::Disconnect(cfg::Disconnect { reason }).encode());
                std::thread::sleep(Duration::from_millis(50));
                break;
            }
            Ok(Out::Close) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        if let Some((id, at)) = sent_at {
            if alive.load(Ordering::Relaxed) == id {
                let ms = i32::try_from(at.elapsed().as_millis()).unwrap_or(i32::MAX);
                let old = latency.load(Ordering::Relaxed);
                latency.store((old * 3 + ms) / 4, Ordering::Relaxed);
                sent_at = None;
            } else if at.elapsed() > KEEP_ALIVE_TIMEOUT {
                let _ = writer.send(
                    &play::Clientbound::Disconnect(cfg::Disconnect {
                        reason: translatable("disconnect.timeout", &[]),
                    })
                    .encode(),
                );
                break;
            }
        }
        if sent_at.is_none() && now >= next {
            let id = i64::try_from(start.elapsed().as_millis()).unwrap_or(0);
            if writer
                .send(&play::Clientbound::KeepAlive(cfg::KeepAlive { id }).encode())
                .is_err()
            {
                break;
            }
            sent_at = Some((id, now));
            next = now + KEEP_ALIVE_EVERY;
        }
    }
    writer.shutdown();
}
