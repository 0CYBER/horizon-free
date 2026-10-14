//! Multiplayer session: a network thread that connects to a server, logs in,
//! runs the configuration phase and then forwards decoded play packets to the
//! game thread. Keep-alives and pings are answered on the network thread like
//! vanilla's connection handlers so a busy frame never times the client out.
//! Online-mode servers need the Microsoft account the launcher signed in
//! (crate::auth): the login joins the server through the session service and
//! encrypts the connection (ClientHandshakePacketListenerImpl.handleHello).
use crate::auth::AuthSession;
use horizon_protocol::{
    PROTOCOL_VERSION,
    connection::{ConnReader, ConnWriter, Connection, Counters},
    crypto::{generate_secret, offline_uuid, server_hash},
    packets::{Empty, configuration as cfg, handshake, login, play},
    registry::Dynamic,
    rsa::RsaPublicKey,
    servers::parse_address,
    text::{Language, plain, translatable},
};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
    thread::JoinHandle,
    time::Duration,
};

/// What the network thread reports to the game.
pub enum Event {
    /// Progress text for the connecting screen.
    Status(String),
    /// Configuration finished: the play state begins with these registries.
    Joined {
        registries: Arc<Dynamic>,
        uuid: u128,
        name: String,
    },
    Packet(Box<play::Clientbound>),
    /// A packet that failed to decode (skipped, like vanilla's skippable packets).
    DecodeError(String),
    /// The server sent us back to configuration and finished it again.
    Reconfigured(Arc<Dynamic>),
    /// The connection ended. `from_server`: the server gave the reason (or
    /// the connection was refused for good), rather than the connection
    /// failing (auto-reconnect retries only failures and shutdowns).
    Disconnected {
        reason: String,
        from_server: bool,
    },
}

pub struct Session {
    events: Receiver<Event>,
    writer: Arc<Mutex<Option<ConnWriter>>>,
    thread: Option<JoinHandle<()>>,
    pub address: String,
    pub name: String,
    pub counters: Arc<Mutex<Option<Arc<Counters>>>>,
    /// HORIZON_NET_DEBUG=1 logs every packet sent from the game thread.
    debug: bool,
}
impl Session {
    /// Start connecting in the background, as `auth`'s account when there is one.
    pub fn connect(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        view_distance: i8,
        lang: Arc<Language>,
    ) -> Self {
        let (send, events) = channel();
        let writer = Arc::new(Mutex::new(None));
        let counters = Arc::new(Mutex::new(None));
        let (thread_writer, thread_counters) = (writer.clone(), counters.clone());
        let (address_owned, name_owned) = (address.to_owned(), name.to_owned());
        let thread = std::thread::Builder::new()
            .name("horizon-net".into())
            .spawn(move || {
                let result = run(
                    &address_owned,
                    &name_owned,
                    auth.as_deref(),
                    view_distance,
                    &lang,
                    &send,
                    &thread_writer,
                    &thread_counters,
                );
                let from_server = result.is_ok();
                let reason = result.unwrap_or_else(|e| e);
                if let Some(w) = thread_writer.lock().unwrap().take() {
                    w.shutdown();
                }
                let _ = send.send(Event::Disconnected {
                    reason,
                    from_server,
                });
            })
            .ok();
        let debug = std::env::var_os("HORIZON_NET_DEBUG").is_some();
        Self {
            events,
            writer,
            thread,
            address: address.to_owned(),
            name: name.to_owned(),
            counters,
            debug,
        }
    }
    /// Everything received since the last poll.
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
    /// Send a play packet; silently dropped once the connection is gone.
    pub fn send(&self, packet: play::Serverbound) {
        if self.debug
            && !matches!(
                packet,
                play::Serverbound::ClientTickEnd(_)
                    | play::Serverbound::MovePlayerPos(_)
                    | play::Serverbound::MovePlayerPosRot(_)
            )
        {
            eprintln!("[net] -> {packet:?}");
        }
        if let Some(w) = self.writer.lock().unwrap().as_mut()
            && w.send(&packet.encode()).is_err()
        {
            w.shutdown();
        }
    }
    pub fn disconnect(&mut self) {
        if let Some(w) = self.writer.lock().unwrap().take() {
            w.shutdown();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// An address of the form `friend:K7QM-3XPD` joins a friend's world through
/// its invite code (horizon-friends): the session goes through a tunnel the
/// client reaches on a loopback port, so everything after is a plain join.
pub const FRIEND: &str = horizon_friends::ADDRESS_PREFIX;

/// An address as players and logs see it: a friend's invite code stays
/// out of it (its secret is in there).
pub fn shown_address(address: &str) -> String {
    match address.strip_prefix(FRIEND) {
        Some(code) => match horizon_friends::invite::parse(code) {
            Some(horizon_friends::invite::Parsed::Short(c)) => {
                format!("Friend's world ({}-****)", c.nameplate)
            }
            _ => "Friend's world".to_owned(),
        },
        None => address.to_owned(),
    }
}

/// The relay the player set (`friendsRelay` in horizon.txt, beside
/// options.txt), else `HORIZON_RELAY_URL`. None by default.
pub fn friends_relay_setting() -> Option<String> {
    let dir = std::env::current_dir().ok()?;
    crate::ui_theme::preference(&dir, "friendsRelay")
        .filter(|r| !r.is_empty())
        .or_else(horizon_friends::relay_url)
}
/// Remember the relay typed on Join a friend (empty forgets it).
pub fn save_friends_relay(relay: &str) {
    if let Ok(dir) = std::env::current_dir() {
        crate::ui_theme::set_preference(&dir, "friendsRelay", (!relay.is_empty()).then_some(relay));
    }
}

/// Reach the friend's world from the code (directly, or through a relay)
/// and open the tunnel: the loopback address to connect to.
fn join_friend(code: &str, send: &Sender<Event>) -> Result<(String, u16), String> {
    use horizon_friends::{
        PathMode,
        guest::{self, GuestConfig},
    };
    status(send, "Finding your friend's world...");
    let tunnel = guest::join(&GuestConfig {
        code: code.to_owned(),
        relay: friends_relay_setting(),
        protocol: PROTOCOL_VERSION,
        game: horizon_protocol::GAME_VERSION.to_owned(),
        path: PathMode::from_env(),
    })
    .map_err(|e| e.to_string())?;
    status(
        send,
        if tunnel.direct {
            "Connected straight to your friend..."
        } else {
            "Connected to your friend through the relay..."
        },
    );
    let (port, _) = tunnel.serve_loopback().map_err(|e| e.to_string())?;
    Ok(("127.0.0.1".to_owned(), port))
}

fn status(send: &Sender<Event>, text: &str) {
    let _ = send.send(Event::Status(text.to_owned()));
}

/// Shown when an online-mode server asks a client without an account to authenticate.
pub const NOT_SIGNED_IN: &str = "This server is in online mode. Sign in with your Microsoft account in the Horizon launcher to join it.";

/// A translation, or the key itself (Language.getOrDefault).
fn tr(lang: &Language, key: &str) -> String {
    lang.get(key).cloned().unwrap_or_else(|| key.to_owned())
}

/// The whole connection. Ok: why it ended for good (the server's reason, or
/// a refusal no retry fixes); Err: the connection failed.
fn run(
    address: &str,
    name: &str,
    auth: Option<&AuthSession>,
    view_distance: i8,
    lang: &Language,
    send: &Sender<Event>,
    shared_writer: &Mutex<Option<ConnWriter>>,
    shared_counters: &Mutex<Option<Arc<Counters>>>,
) -> Result<String, String> {
    let (host, port) = match address.strip_prefix(FRIEND) {
        Some(code) => join_friend(code, send)?,
        None => {
            let Some(target) = parse_address(address) else {
                return Ok(format!("Invalid server address: {address}"));
            };
            status(send, "Connecting to the server...");
            target
        }
    };
    let mut c =
        Connection::connect(&host, port, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    *shared_counters.lock().unwrap() = Some(c.reader.counters.clone());
    let e = |e: horizon_protocol::Error| e.to_string();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: host.clone(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .map_err(e)?;
    // Vanilla's login sends the account's profile id, or the offline one.
    let uuid = auth.map_or_else(|| offline_uuid(name), |a| a.uuid);
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.to_owned(),
            uuid,
        })
        .encode(),
    )
    .map_err(e)?;
    let profile = loop {
        let (id, payload) = c.recv().map_err(e)?;
        match login::Clientbound::decode(id, &payload).map_err(e)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(p) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .map_err(e)?;
                break p.profile;
            }
            login::Clientbound::Disconnect(p) => {
                let reason = serde_json::from_str::<serde_json::Value>(&p.reason_json)
                    .map(|v| json_text(&v, lang))
                    .unwrap_or(p.reason_json);
                return Ok(reason);
            }
            login::Clientbound::Hello(hello) => {
                status(send, &tr(lang, "connect.authorizing"));
                let key = RsaPublicKey::from_der(&hello.public_key)
                    .map_err(|err| format!("Protocol error: {err}"))?;
                let secret = generate_secret().map_err(e)?;
                if hello.should_authenticate {
                    let account = auth.ok_or_else(|| NOT_SIGNED_IN.to_owned())?;
                    let hash = server_hash(&hello.server_id, &secret, &hello.public_key);
                    if let Err(err) = crate::auth::join_server(account, &hash) {
                        let reason = match err.reason() {
                            Ok(key) => tr(lang, key),
                            Err(message) => message.to_owned(),
                        };
                        return Err(plain(
                            &translatable("disconnect.loginFailedInfo", &[&reason]),
                            lang,
                        ));
                    }
                }
                status(send, &tr(lang, "connect.encrypting"));
                c.send(
                    &login::Serverbound::Key(login::Key {
                        key: key.encrypt(&secret).map_err(e)?,
                        challenge: key.encrypt(&hello.challenge).map_err(e)?,
                    })
                    .encode(),
                )
                .map_err(e)?;
                c.enable_encryption(&secret);
            }
            login::Clientbound::CustomQuery(q) => {
                c.send(
                    &login::Serverbound::CustomQueryAnswer(login::CustomQueryAnswer {
                        transaction: q.transaction,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(e)?;
            }
            login::Clientbound::CookieRequest(q) => {
                c.send(
                    &login::Serverbound::CookieResponse(login::CookieResponse {
                        key: q.key,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(e)?;
            }
            login::Clientbound::Unknown(..) => {}
        }
    };
    status(send, "Joining world...");
    c.send(
        &cfg::Serverbound::ClientInformation(cfg::ClientInformation::new(view_distance)).encode(),
    )
    .map_err(e)?;
    c.send(
        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: brand(),
        })
        .encode(),
    )
    .map_err(e)?;
    let (mut reader, writer) = c.split();
    *shared_writer.lock().unwrap() = Some(writer);
    let registries = configure(&mut reader, shared_writer, lang)?;
    let _ = send.send(Event::Joined {
        registries: Arc::new(registries),
        uuid: profile.uuid,
        name: profile.name,
    });
    loop {
        let (id, payload) = reader.recv().map_err(e)?;
        let packet = match play::Clientbound::decode(id, &payload) {
            Ok(p) => p,
            Err(err) => {
                let name = horizon_protocol::ids::name(
                    horizon_protocol::State::Play,
                    horizon_protocol::Direction::Clientbound,
                    id,
                )
                .unwrap_or("?");
                let _ = send.send(Event::DecodeError(format!("{name}: {err}")));
                continue;
            }
        };
        let reply = |p: play::Serverbound| {
            if let Some(w) = shared_writer.lock().unwrap().as_mut() {
                let _ = w.send(&p.encode());
            }
        };
        match packet {
            play::Clientbound::KeepAlive(k) => reply(play::Serverbound::KeepAlive(k)),
            play::Clientbound::Ping(p) => reply(play::Serverbound::Pong(p)),
            play::Clientbound::BundleDelimiter(_) => {}
            play::Clientbound::ResourcePackPush(p) => {
                // Declined: Horizon has no server resource packs.
                reply(play::Serverbound::ResourcePack(cfg::ResourcePackResponse {
                    id: p.id,
                    action: 1,
                }));
            }
            play::Clientbound::StartConfiguration(_) => {
                reply(play::Serverbound::ConfigurationAcknowledged(Empty));
                let registries = configure(&mut reader, shared_writer, lang)?;
                let _ = send.send(Event::Reconfigured(Arc::new(registries)));
            }
            play::Clientbound::Disconnect(d) => return Ok(plain(&d.reason, lang)),
            other => {
                if send.send(Event::Packet(Box::new(other))).is_err() {
                    return Ok("Disconnected".into());
                }
            }
        }
    }
}

/// The configuration phase: registries, tags and known packs until the
/// server finishes it (ClientConfigurationPacketListenerImpl).
fn configure(
    reader: &mut ConnReader,
    writer: &Mutex<Option<ConnWriter>>,
    lang: &Language,
) -> Result<Dynamic, String> {
    let mut registries = Dynamic::default();
    let reply = |p: cfg::Serverbound| -> Result<(), String> {
        match writer.lock().unwrap().as_mut() {
            Some(w) => w.send(&p.encode()).map_err(|e| e.to_string()),
            None => Err("Disconnected".into()),
        }
    };
    loop {
        let (id, payload) = reader.recv().map_err(|e| e.to_string())?;
        match cfg::Clientbound::decode(id, &payload).map_err(|e| e.to_string())? {
            cfg::Clientbound::KeepAlive(k) => reply(cfg::Serverbound::KeepAlive(k))?,
            cfg::Clientbound::Ping(p) => reply(cfg::Serverbound::Pong(p))?,
            // Know no packs: the server then sends every registry entry in full.
            cfg::Clientbound::SelectKnownPacks(_) => {
                reply(cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks {
                    packs: vec![],
                }))?
            }
            cfg::Clientbound::RegistryData(r) => registries.add(r),
            cfg::Clientbound::CodeOfConduct(_) => {
                reply(cfg::Serverbound::AcceptCodeOfConduct(Empty))?
            }
            cfg::Clientbound::ResourcePackPush(p) => {
                reply(cfg::Serverbound::ResourcePack(cfg::ResourcePackResponse {
                    id: p.id,
                    action: 1,
                }))?
            }
            cfg::Clientbound::CookieRequest(q) => {
                reply(cfg::Serverbound::CookieResponse(cfg::CookieResponse {
                    key: q.key,
                    payload: None,
                }))?
            }
            cfg::Clientbound::Disconnect(d) => return Err(plain(&d.reason, lang)),
            cfg::Clientbound::FinishConfiguration(_) => {
                reply(cfg::Serverbound::FinishConfiguration(Empty))?;
                return Ok(registries);
            }
            _ => {}
        }
    }
}

/// The minecraft:brand payload: a string naming the client.
fn brand() -> Vec<u8> {
    let mut w = horizon_protocol::buf::Writer::new();
    w.string("horizon");
    w.data
}

/// Flatten a JSON chat component (login disconnects are JSON).
pub fn json_text(v: &serde_json::Value, lang: &Language) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().map(|e| json_text(e, lang)).collect(),
        serde_json::Value::Object(o) => {
            let mut out = if let Some(t) = o.get("text").and_then(|t| t.as_str()) {
                t.to_owned()
            } else if let Some(key) = o.get("translate").and_then(|t| t.as_str()) {
                let args: Vec<String> = o
                    .get("with")
                    .and_then(|w| w.as_array())
                    .map(|a| a.iter().map(|e| json_text(e, lang)).collect())
                    .unwrap_or_default();
                let pattern = lang
                    .get(key)
                    .cloned()
                    .or_else(|| {
                        o.get("fallback")
                            .and_then(|f| f.as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| key.to_owned());
                horizon_protocol::text::format_java(&pattern, &args)
            } else {
                String::new()
            };
            if let Some(extra) = o.get("extra").and_then(|e| e.as_array()) {
                for e in extra {
                    out.push_str(&json_text(e, lang));
                }
            }
            out
        }
        other => other.to_string(),
    }
}

/// en_us.json from the prepared resources.
pub fn load_language(resources: &std::path::Path) -> Language {
    std::fs::read(resources.join("assets/minecraft/lang/en_us.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Ask a server for its status (ServerStatusPinger): MOTD, players and the
/// round trip time in milliseconds.
pub fn ping(address: &str, timeout: Duration) -> Result<(serde_json::Value, u128), String> {
    use horizon_protocol::packets::status;
    let (host, port) = parse_address(address).ok_or("Invalid address")?;
    let mut c = Connection::connect(&host, port, timeout).map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let e = |e: horizon_protocol::Error| e.to_string();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: host.clone(),
            port,
            intent: handshake::STATUS,
        })
        .encode(),
    )
    .map_err(e)?;
    c.send(&status::Serverbound::StatusRequest(Empty).encode())
        .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    let json = match status::Clientbound::decode(id, &payload).map_err(e)? {
        status::Clientbound::StatusResponse(s) => {
            serde_json::from_str(&s.json).map_err(|e| e.to_string())?
        }
        _ => return Err("unexpected status reply".into()),
    };
    let start = std::time::Instant::now();
    c.send(&status::Serverbound::PingRequest(status::Ping { time: 1 }).encode())
        .map_err(e)?;
    let _ = c.recv().map_err(e)?;
    Ok((json, start.elapsed().as_millis()))
}
