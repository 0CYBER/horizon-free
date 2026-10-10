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
/// Connection.setupCompression threshold of the integrated server.
const COMPRESSION_THRESHOLD: i32 = 256;

/// Vanilla's server-side disconnect during login is a JSON component.
fn login_disconnect(c: &mut Connection, key: &str) {
    let json = format!("{{\"translate\":\"{key}\"}}");
    let _ =
        c.send(&login::Clientbound::Disconnect(login::Disconnect { reason_json: json }).encode());
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

pub fn serve(stream: TcpStream, shared: Arc<Shared>) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
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
        handshake::STATUS => serve_status(&mut c, &shared),
        handshake::LOGIN | handshake::TRANSFER => {
            if intention.protocol != PROTOCOL_VERSION {
                let key = if intention.protocol < PROTOCOL_VERSION {
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
            serve_login(c, &shared)
        }
        other => Err(format!("unknown intent {other}")),
    }
}

fn serve_status(c: &mut Connection, shared: &Shared) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match status::Serverbound::decode(id, &payload).map_err(e)? {
            status::Serverbound::StatusRequest(_) => {
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

fn serve_login(mut c: Connection, shared: &Arc<Shared>) -> Result<(), String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let (id, payload) = c.recv().map_err(e)?;
    let hello = match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::Hello(h) => h,
        _ => return Err("expected a login hello".into()),
    };
    if !valid_name(&hello.name) {
        login_disconnect(&mut c, "multiplayer.disconnect.invalid_name");
        return Ok(());
    }
    // Offline mode: the profile is derived from the name (PlayerList.canPlayerLogin
    // then rejects duplicates and a full server).
    let uuid = offline_uuid(&hello.name);
    if let Some(reason) = shared.try_admit(uuid, &hello.name) {
        login_disconnect(&mut c, reason);
        return Ok(());
    }
    let result = login_and_configure(&mut c, shared, uuid, &hello.name);
    match result {
        Ok(Some(parts)) => {
            play(c, shared, parts);
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
    c.send(
        &login::Clientbound::Compression(login::Compression {
            threshold: COMPRESSION_THRESHOLD,
        })
        .encode(),
    )
    .map_err(e)?;
    c.set_compression(COMPRESSION_THRESHOLD);
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
                w.string("horizon");
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

fn play(c: Connection, shared: &Arc<Shared>, parts: Parts) {
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
