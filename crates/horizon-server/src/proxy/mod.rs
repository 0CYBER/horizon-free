//! `[proxy]`: the public port belongs to the supervisor, and the game listens on 127.0.0.1 behind it.
//!
//! The proxy logs each player in itself (handshake, status, online-mode login with its encryption and
//! compression: auth.rs), then logs into the game as that player through Velocity modern forwarding
//! (backend.rs) and relays packets both ways (session.rs). It keeps what a later login to the game must be
//! told again (replay.rs). Not a general proxy: one game, the one this supervisor runs.

mod auth;
mod backend;
#[cfg(test)]
mod join_tests;
mod replay;
mod session;
mod wire;

use horizon_server_launch::config::Config;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use wire::{Connection, handshake, status};

/// The secret the proxy signs forwarded players with, beside `server.properties`: the file the game reads.
const SECRET_FILE: &str = "forwarding.secret";
/// Where the proxy keeps the secret it made, so it can tell that file from one an owner put there.
const OWN_SECRET: &str = ".horizon/proxy-forwarding.secret";
/// How long a client may take over its handshake and login.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound a client write so holding or releasing this session cannot stall the rest of the proxy.
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// How often held players are looked after.
const TICK: Duration = if cfg!(test) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(1)
};

pub struct Inner {
    /// The game, on this machine.
    pub game: SocketAddr,
    pub secret: Vec<u8>,
    /// The threshold the proxy compresses its clients' connections with.
    pub compression: i32,
    keys: Option<auth::Keys>,
    status_enabled: bool,
    /// What the game is doing for the players: see [`Gate`].
    gate: Mutex<GameGate>,
    /// How long a held player waits for the game before it is sent away.
    hold: Duration,
    /// Every player through the proxy.
    pub sessions: Mutex<Vec<Arc<session::Session>>>,
    log: Mutex<Box<dyn Fn(&str, &str) + Send>>,
    /// The connections still before login: see [`Admission`].
    admission: Admission,
}

/// Whether the game takes logins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    /// Not yet, or not for now (it starts, restarts or crashed): players that come are held.
    Holding,
    Up,
    /// A stop for good has begun: the game's kicks reach the players as they are.
    Closing,
}

struct GameGate {
    phase: Gate,
    generation: u64,
    pid: Option<u32>,
}

impl Inner {
    fn gate(&self) -> std::sync::MutexGuard<'_, GameGate> {
        self.gate.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_up(&self) -> bool {
        self.gate().phase == Gate::Up
    }

    fn up_generation(&self) -> Option<u64> {
        let gate = self.gate();
        (gate.phase == Gate::Up).then_some(gate.generation)
    }

    pub fn closing(&self) -> bool {
        self.gate().phase == Gate::Closing
    }

    /// A game that stops itself sends everyone its shutdown kick: unless the stop is for good, that is the
    /// start of a hold, not something to pass on.
    pub fn hold_on_shutdown(&self) -> bool {
        !self.closing()
    }

    fn snapshot(&self) -> Vec<Arc<session::Session>> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The game is going away (a restart, a crash): every player is held, and the ones that come.
    fn stopping(&self, pid: Option<u32>) {
        let generation = {
            let mut gate = self.gate();
            if gate.phase != Gate::Up || (pid.is_some() && pid != gate.pid) {
                return;
            }
            gate.phase = Gate::Holding;
            gate.generation
        };
        for s in self.snapshot() {
            s.hold_game(generation);
        }
    }

    fn game_up(&self) {
        {
            let mut gate = self.gate();
            if gate.phase != Gate::Holding {
                return;
            }
            gate.phase = Gate::Up;
            gate.generation += 1;
        }
        for s in self.snapshot() {
            s.resume();
        }
    }

    /// The game stopped and will not be started again: the players are sent away.
    fn release_all(&self) {
        self.gate().phase = Gate::Holding;
        let reason = wire::translated("horizon.proxy.stopped", "The server was stopped");
        for s in self.snapshot() {
            s.disconnect(&reason);
        }
    }

    pub fn note(&self, level: &str, line: &str) {
        (self.log.lock().unwrap_or_else(|e| e.into_inner()))(level, line);
    }
}

/// The running proxy: its listener's thread stays until the supervisor ends.
#[derive(Clone)]
pub struct Proxy {
    inner: Arc<Inner>,
}

/// Starts listening on the public address. `dir` is the server folder; `log` receives (level, line).
pub fn start(
    dir: &Path,
    config: &Config,
    log: impl Fn(&str, &str) + Send + 'static,
) -> Result<Proxy, String> {
    let bind: IpAddr = if config.server.bind.is_empty() {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        config
            .server
            .bind
            .parse()
            .map_err(|_| format!("server.bind {:?} is not an IP address", config.server.bind))?
    };
    let listener = TcpListener::bind((bind, config.server.port))
        .map_err(|e| format!("[proxy] can't listen on {bind}:{}: {e}", config.server.port))?;
    let secret = secret(dir)?;
    let keys = if config.server.online_mode {
        Some(auth::Keys::generate()?)
    } else {
        None
    };
    let inner = Arc::new(Inner {
        game: SocketAddr::from((Ipv4Addr::LOCALHOST, config.proxy.game_port)),
        secret,
        compression: compression_threshold(config, dir),
        keys,
        status_enabled: !config
            .effective_property(dir, "enable-status")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("false")),
        gate: Mutex::new(GameGate {
            phase: Gate::Holding,
            generation: 0,
            pid: None,
        }),
        hold: Duration::from_secs(u64::from(config.proxy.hold_seconds)),
        sessions: Mutex::new(Vec::new()),
        log: Mutex::new(Box::new(log)),
        admission: Admission::new(config.server.max_players as usize),
    });
    let accepting = inner.clone();
    thread::Builder::new()
        .name("Proxy listener".into())
        .spawn(move || accept(listener, &accepting))
        .map_err(|e| e.to_string())?;
    let ticking = inner.clone();
    thread::Builder::new()
        .name("Proxy keep-alive".into())
        .spawn(move || {
            loop {
                thread::sleep(TICK);
                let now = std::time::Instant::now();
                for s in ticking.snapshot() {
                    s.tick(now, ticking.hold);
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Proxy { inner })
}

impl Proxy {
    /// The process whose console can announce that this game is stopping.
    pub fn game_starting(&self, pid: u32) {
        self.inner.gate().pid = Some(pid);
    }

    /// The game answers: held players are logged into it again, and new ones go straight in.
    pub fn game_up(&self) {
        self.inner.game_up();
    }

    /// The game is about to stop, or has stopped by itself, and will be started again: hold everyone.
    pub fn game_stopping(&self) {
        self.inner.stopping(None);
    }

    pub fn game_stopping_if_current(&self, pid: u32) {
        self.inner.stopping(Some(pid));
    }

    /// A stop for good has begun: what the game says to the players reaches them.
    pub fn closing(&self) {
        self.inner.gate().phase = Gate::Closing;
    }

    /// The game's process ended. `for_good`: nothing starts it again, so the players are sent away.
    pub fn game_exited(&self, for_good: bool) {
        self.inner.gate().pid = None;
        if for_good {
            self.inner.release_all();
        } else {
            self.inner.stopping(None);
        }
    }
}

/// `network-compression-threshold` as the owner set it, vanilla's 256 otherwise.
fn compression_threshold(config: &Config, dir: &Path) -> i32 {
    config
        .effective_property(dir, "network-compression-threshold")
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(256)
}

/// The shared secret: made once and kept, and put where the game reads it. A `forwarding.secret` the owner
/// wrote for a Velocity of their own is never overwritten: that and the proxy can't both be in front of the game.
fn secret(dir: &Path) -> Result<Vec<u8>, String> {
    let (own, file): (PathBuf, PathBuf) = (dir.join(OWN_SECRET), dir.join(SECRET_FILE));
    let kept = std::fs::read(&own)
        .ok()
        .map(|s| s.trim_ascii().to_vec())
        .filter(|s| !s.is_empty());
    let secret = match kept {
        Some(s) => s,
        None => {
            let mut random = [0u8; 24];
            getrandom::fill(&mut random)
                .map_err(|e| format!("no random numbers for the proxy's secret: {e}"))?;
            let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
            if let Some(parent) = own.parent() {
                private_dir(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            write_private(&own, hex.as_bytes()).map_err(|e| format!("{}: {e}", own.display()))?;
            hex.into_bytes()
        }
    };
    match std::fs::read(&file) {
        Ok(existing) if existing.trim_ascii() != secret.as_slice() => {
            return Err(format!(
                "[proxy] is on but {SECRET_FILE} holds another secret (a Velocity's?): remove the file, or turn [proxy] off"
            ));
        }
        Ok(_) => {
            // A file an earlier start left with looser permissions.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
                let _ = std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o600));
            }
        }
        Err(_) => write_private(&file, &secret).map_err(|e| format!("{}: {e}", file.display()))?,
    }
    Ok(secret)
}

/// The folder the proxy keeps its own secret in: readable by its owner only (0700 on Unix).
fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes a secret so only its owner can read it (0600 on Unix, from the first byte: the file is created
/// with that mode, and a file already there is tightened).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    options.open(path)?.write_all(bytes)
}

/// With `[proxy]` off again, the file the proxy put there goes, or the game would refuse everyone who joins
/// without a proxy.
pub fn remove_own_secret(dir: &Path) {
    let own = std::fs::read(dir.join(OWN_SECRET)).ok();
    let file = dir.join(SECRET_FILE);
    if let (Some(own), Ok(existing)) = (own, std::fs::read(&file))
        && own.trim_ascii() == existing.trim_ascii()
    {
        let _ = std::fs::remove_file(file);
    }
}

/// Connections from one source (an IPv6 /64 counts as one) still before login at once, as the game's own
/// per-address limit.
const PER_SOURCE: usize = 10;
/// Connections before login beyond the player limit, as the game's spare places for logins and pings.
const SPARE_CONNECTIONS: usize = 64;

/// Who counts as one source: an IPv4 address, or an IPv6 /64 (one subscriber holds a whole prefix).
fn source(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & !(u64::MAX as u128))),
        v4 => v4,
    }
}

/// Whether many players may come from `ip`: a tunnel, a container's port forward or the local network
/// hands them all over from one address.
fn shared_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

/// Caps the connections that have not logged in yet, so silent ones can't take a thread each without end.
/// Players that logged in are not counted: they are the game's players and its own limits apply.
struct Admission {
    open: Arc<std::sync::atomic::AtomicUsize>,
    by_source: Arc<Mutex<HashMap<IpAddr, usize>>>,
    most: usize,
}

impl Admission {
    fn new(max_players: usize) -> Self {
        Self {
            open: Arc::default(),
            by_source: Arc::default(),
            most: max_players.saturating_add(SPARE_CONNECTIONS),
        }
    }

    /// A place for a connection from `ip`, or None when its source or the proxy has its most.
    fn take(&self, ip: IpAddr) -> Option<Place> {
        // Judge the address itself: grouping an IPv6 address into its /64 turns `::1` into `::`,
        // which is no longer loopback.
        let exempt = shared_address(ip.to_canonical());
        let key = source(ip);
        let mut by = self.by_source.lock().unwrap_or_else(|e| e.into_inner());
        if !exempt && by.get(&key).copied().unwrap_or(0) >= PER_SOURCE {
            return None;
        }
        let slot = crate::http::Slot::take(&self.open, self.most)?;
        if !exempt {
            *by.entry(key).or_default() += 1;
        }
        Some(Place {
            _slot: slot,
            by_source: self.by_source.clone(),
            counted: (!exempt).then_some(key),
        })
    }
}

/// A connection's place before login, given back when dropped.
struct Place {
    _slot: crate::http::Slot,
    by_source: Arc<Mutex<HashMap<IpAddr, usize>>>,
    counted: Option<IpAddr>,
}

impl Drop for Place {
    fn drop(&mut self) {
        let Some(key) = self.counted else { return };
        let mut by = self.by_source.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = by.get_mut(&key) {
            *n -= 1;
            if *n == 0 {
                by.remove(&key);
            }
        }
    }
}

fn accept(listener: TcpListener, inner: &Arc<Inner>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let Some(place) = stream
            .peer_addr()
            .ok()
            .and_then(|a| inner.admission.take(a.ip()))
        else {
            // Closed before it costs a thread.
            continue;
        };
        let inner = inner.clone();
        let _ = thread::Builder::new()
            .name("Proxy client".into())
            .spawn(move || {
                if let Err(e) = serve(stream, &inner, place) {
                    inner.note("DEBUG", &format!("A connection to the proxy ended: {e}"));
                }
            });
    }
}

fn serve(stream: TcpStream, inner: &Arc<Inner>, place: Place) -> Result<(), String> {
    stream
        .set_write_timeout(Some(CLIENT_WRITE_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let peer = stream.peer_addr().map_err(|e| e.to_string())?;
    let mut c = Connection::from_stream(stream).map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|e| e.to_string())?;
    // The timeout above is per read: this one ends the whole handshake and login.
    let deadline = Deadline::start(&c, HANDSHAKE_TIMEOUT, place)?;
    let e = |e: wire::Error| e.to_string();
    let (id, payload) = c.recv().map_err(e)?;
    let hello =
        handshake::decode_intention(id, &payload, handshake::PROXY_HOST_LIMIT).map_err(e)?;
    match hello.intent {
        handshake::STATUS if !inner.status_enabled => Ok(()),
        handshake::STATUS => serve_status(&mut c, inner, hello.protocol),
        handshake::LOGIN | handshake::TRANSFER => {
            if hello.protocol != wire::PROTOCOL_VERSION {
                let reason = wire::translated_json(
                    "multiplayer.disconnect.incompatible",
                    "Incompatible client! Please use the matching version",
                );
                let _ = c.send(
                    &wire::login::Clientbound::Disconnect(wire::login::Disconnect {
                        reason_json: reason,
                    })
                    .encode(),
                );
                return Ok(());
            }
            serve_login(c, inner, peer.ip(), hello.intent, deadline)
        }
        other => Err(format!("unknown handshake intent {other}")),
    }
}

/// Closes a connection that is still in its handshake or login when the time is up; stops when dropped.
struct Deadline {
    done: Arc<AtomicBool>,
    watcher: thread::Thread,
    /// Held until login is over: [`Admission`] counts only connections before it.
    _place: Place,
}

impl Deadline {
    fn start(c: &Connection, limit: Duration, place: Place) -> Result<Deadline, String> {
        let closer = c.reader.closer().map_err(|e| e.to_string())?;
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let watcher = thread::Builder::new()
            .name("Proxy deadline".into())
            .spawn(move || {
                let end = std::time::Instant::now() + limit;
                while !flag.load(Ordering::SeqCst) {
                    let Some(left) = end.checked_duration_since(std::time::Instant::now()) else {
                        closer.close();
                        return;
                    };
                    thread::park_timeout(left);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Deadline {
            done,
            watcher: watcher.thread().clone(),
            _place: place,
        })
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.watcher.unpark();
    }
}

fn serve_login(
    mut c: Connection,
    inner: &Arc<Inner>,
    peer: IpAddr,
    intent: i32,
    deadline: Deadline,
) -> Result<(), String> {
    let e = |e: wire::Error| e.to_string();
    let (id, payload) = c.recv().map_err(e)?;
    let name = match wire::login::Serverbound::decode(id, &payload).map_err(e)? {
        wire::login::Serverbound::Hello(h) => h.name,
        _ => return Err("expected a login hello".into()),
    };
    if !auth::valid_name(&name) {
        return Err(format!("{name:?} is not a player name"));
    }
    match auth::profile(&mut c, inner.keys.as_ref(), &name)? {
        Ok(who) => {
            session::run(inner, c, who, peer, intent, deadline);
            Ok(())
        }
        Err(reason_json) => {
            let _ = c.send(
                &wire::login::Clientbound::Disconnect(wire::login::Disconnect { reason_json })
                    .encode(),
            );
            Ok(())
        }
    }
}

/// The server list: the game's own answer, or one that says it is restarting.
fn serve_status(c: &mut Connection, inner: &Arc<Inner>, protocol: i32) -> Result<(), String> {
    let e = |e: wire::Error| e.to_string();
    // Like vanilla: one status request, then pings.
    let mut answered = false;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match status::Serverbound::decode(id, &payload).map_err(e)? {
            status::Serverbound::StatusRequest(_) if answered => {
                return Err("a second status request".into());
            }
            status::Serverbound::StatusRequest(_) => {
                answered = true;
                let json = inner
                    .is_up()
                    .then(|| backend::status_json(inner.game).ok())
                    .flatten()
                    .unwrap_or_else(|| restarting_status(protocol));
                c.send(
                    &status::Clientbound::StatusResponse(status::StatusResponse { json }).encode(),
                )
                .map_err(e)?;
            }
            status::Serverbound::PingRequest(ping) => {
                c.send(&status::Clientbound::PongResponse(ping).encode())
                    .map_err(e)?;
                return Ok(());
            }
            status::Serverbound::Unknown(..) => return Err("unknown status packet".into()),
        }
    }
}

/// What the server list shows while the game is down.
fn restarting_status(protocol: i32) -> String {
    serde_json::json!({
        "version": { "name": wire::version_name(), "protocol": protocol },
        "players": { "max": 0, "online": 0 },
        "description": { "translate": "horizon.proxy.restarting", "fallback": "Restarting, back in a moment" },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_restarting_status_is_translated_and_valid_json() {
        let v: serde_json::Value = serde_json::from_str(&restarting_status(777)).unwrap();
        assert_eq!(v["description"]["translate"], "horizon.proxy.restarting");
        assert_eq!(v["version"]["protocol"], 777);
    }

    #[test]
    fn a_secret_that_is_not_ours_stops_the_start_and_ours_is_kept() {
        let dir = std::env::temp_dir().join(format!("horizon-proxy-secret-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SECRET_FILE), "velocitys-own").unwrap();
        assert!(secret(&dir).unwrap_err().contains("another secret"));
        std::fs::remove_file(dir.join(SECRET_FILE)).unwrap();
        let first = secret(&dir).unwrap();
        assert_eq!(secret(&dir).unwrap(), first, "kept across starts");
        assert_eq!(std::fs::read(dir.join(SECRET_FILE)).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                |p: std::path::PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(dir.join(SECRET_FILE)), 0o600);
            assert_eq!(mode(dir.join(OWN_SECRET)), 0o600);
            assert_eq!(mode(dir.join(".horizon")), 0o700);
        }
        remove_own_secret(&dir);
        assert!(!dir.join(SECRET_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_source_has_its_share_of_places_an_ipv6_prefix_counts_as_one_and_places_come_back() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let admission = Admission::new(100);
        let mut held: Vec<Place> = (0..PER_SOURCE)
            .map(|_| admission.take(ip("203.0.113.7")).unwrap())
            .collect();
        assert!(admission.take(ip("203.0.113.7")).is_none());
        assert!(admission.take(ip("198.51.100.2")).is_some());
        // Local addresses (a tunnel or a container hands everyone over from one) have no share.
        let local: Vec<Place> = (0..PER_SOURCE + 5)
            .map(|_| admission.take(ip("127.0.0.1")).unwrap())
            .collect();
        let v6: Vec<Place> = (0..PER_SOURCE)
            .map(|i| admission.take(ip(&format!("2001:db8:1:2::{i:x}"))).unwrap())
            .collect();
        assert!(admission.take(ip("2001:db8:1:2:ffff::1")).is_none());
        assert!(admission.take(ip("2001:db8:1:3::1")).is_some());
        let loopback6: Vec<Place> = (0..PER_SOURCE + 5)
            .map(|_| admission.take(ip("::1")).unwrap())
            .collect();
        held.pop();
        assert!(admission.take(ip("203.0.113.7")).is_some());
        drop((held, local, v6, loopback6));
        assert_eq!(admission.open.load(Ordering::SeqCst), 0);
        assert!(admission.by_source.lock().unwrap().is_empty());
    }
}
