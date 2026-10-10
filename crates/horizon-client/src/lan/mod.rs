//! Hosting a world on the local network. The network side lives here: the
//! listener, the per-connection handshake/login/configuration threads and the
//! writer threads (`conn`), plus the data the configuration phase sends
//! (`data`). The play state is driven by the game thread (`hub`), which only
//! ever exchanges decoded packets with these threads through channels, so a
//! world that is not open to the network pays nothing.
pub mod chunks;
pub mod conn;
pub mod containers;
pub mod data;
pub mod discovery;
pub mod entity;
pub mod guest;
pub mod hub;
pub mod menu;
pub mod playerdata;

pub use conn::PlayerSender;
use horizon_protocol::packets::play;
use horizon_protocol::{GAME_VERSION, PROTOCOL_VERSION, packets::configuration::ClientInformation};
use std::{
    collections::HashSet,
    io,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::JoinHandle,
    time::Duration,
};

/// IntegratedServer.MAX_PLAYERS.
pub const MAX_PLAYERS: usize = 8;

/// Messages from the connection threads to the game thread.
pub enum NetEvent {
    /// A client finished configuration and enters the play state.
    Joined(Box<Joined>),
    Packet(u32, Box<play::Serverbound>),
    /// The connection ended (reason for the log).
    Left(u32, String),
}
pub struct Joined {
    pub conn: u32,
    pub uuid: u128,
    pub name: String,
    pub info: ClientInformation,
    pub brand: String,
    pub sender: PlayerSender,
    /// Smoothed keep-alive round trip in milliseconds.
    pub latency: Arc<AtomicI32>,
}
/// Queued for a connection's writer thread.
pub enum Out {
    /// A packet body: VarInt id and payload.
    Packet(Vec<u8>),
    /// Send a play disconnect with this component, then close.
    Disconnect(horizon_world::nbt::Tag),
    Close,
}

/// State shared with the connection threads.
pub struct Shared {
    pub sync: Arc<data::SyncData>,
    pub events: Sender<NetEvent>,
    motd: Mutex<String>,
    online: Mutex<HashSet<u128>>,
    max_players: usize,
    next: AtomicU32,
    stop: AtomicBool,
}
impl Shared {
    pub fn next_id(&self) -> u32 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
    /// PlayerList.canPlayerLogin: duplicates and a full server are refused.
    pub fn try_admit(&self, uuid: u128, _name: &str) -> Option<&'static str> {
        let mut online = self.online.lock().unwrap();
        if online.contains(&uuid) {
            return Some("multiplayer.disconnect.duplicate_login");
        }
        if online.len() >= self.max_players {
            return Some("multiplayer.disconnect.server_full");
        }
        online.insert(uuid);
        None
    }
    pub fn release(&self, uuid: u128) {
        self.online.lock().unwrap().remove(&uuid);
    }
    pub fn online(&self) -> usize {
        self.online.lock().unwrap().len()
    }
    /// The status response (ServerStatus): version, players and the MOTD.
    pub fn status_json(&self) -> String {
        let motd = self.motd.lock().unwrap().clone();
        serde_json::json!({
            "version": { "name": GAME_VERSION, "protocol": PROTOCOL_VERSION },
            "players": { "max": self.max_players, "online": self.online() },
            "description": { "text": motd },
            "enforcesSecureChat": false,
        })
        .to_string()
    }
}

/// A running LAN server.
pub struct Lan {
    pub shared: Arc<Shared>,
    events: Receiver<NetEvent>,
    pub port: u16,
    listener: Option<JoinHandle<()>>,
    /// The multicast announcement (LanServerPinger); absent when the socket failed.
    _pinger: Option<discovery::Pinger>,
}
impl Lan {
    /// Listen on `port` (0 picks a free one).
    pub fn start(
        sync: Arc<data::SyncData>,
        port: u16,
        motd: String,
        max_players: usize,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let _pinger = discovery::Pinger::start(motd.clone(), port).ok();
        let (tx, events) = channel();
        let shared = Arc::new(Shared {
            sync,
            events: tx,
            motd: Mutex::new(motd),
            online: Mutex::new(HashSet::new()),
            max_players,
            next: AtomicU32::new(1),
            stop: AtomicBool::new(false),
        });
        let accept_shared = shared.clone();
        let listener = std::thread::Builder::new()
            .name("horizon-lan-accept".into())
            .spawn(move || accept_loop(listener, accept_shared))?;
        Ok(Self {
            shared,
            events,
            port,
            listener: Some(listener),
            _pinger,
        })
    }
    /// Everything the connection threads reported since the last call.
    pub fn poll(&self) -> Vec<NetEvent> {
        self.events.try_iter().collect()
    }
    /// Reserve a uuid for the host so a guest cannot take the name.
    pub fn reserve(&self, uuid: u128) {
        self.shared.online.lock().unwrap().insert(uuid);
    }
    pub fn set_motd(&self, motd: &str) {
        *self.shared.motd.lock().unwrap() = motd.to_owned();
    }
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.listener.take() {
            let _ = t.join();
        }
        self._pinger = None;
    }
}
impl Drop for Lan {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, addr)) => {
                let shared = shared.clone();
                let _ = std::thread::Builder::new()
                    .name("horizon-lan-conn".into())
                    .spawn(move || handle(stream, addr, shared));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
fn handle(stream: TcpStream, addr: SocketAddr, shared: Arc<Shared>) {
    let _ = stream.set_nonblocking(false);
    if let Err(e) = conn::serve(stream, shared)
        && std::env::var_os("HORIZON_NET_DEBUG").is_some()
    {
        eprintln!("[lan] {addr}: {e}");
    }
}

/// HttpUtil.getAvailablePort: a free local port.
pub fn available_port() -> u16 {
    TcpListener::bind(("0.0.0.0", 0))
        .and_then(|l| l.local_addr())
        .map_or(25565, |a| a.port())
}
