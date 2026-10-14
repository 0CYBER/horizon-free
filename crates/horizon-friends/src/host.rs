//! The host's end: registers with the relay for a code and keeps that
//! control connection open; for each guest the relay announces, attaches a
//! new connection, agrees on the key, settles on a path and connects the
//! session to the integrated server's port. Everything runs on its own
//! threads; the game polls [`Host::poll`] and [`Host::state`].
use crate::{
    Code, PathMode,
    crypto::{self, Role},
    direct, resolve_relay, session,
    wire::{self, FromRelay, ToRelay},
};
use std::{
    collections::HashMap,
    io::{self, BufReader},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Wrong codes tried before the host retires its code for a new one.
pub const MAX_WRONG: u32 = 5;
/// Guests that can be connecting at once (IntegratedServer.MAX_PLAYERS).
pub const MAX_SESSIONS: usize = 8;

#[derive(Clone)]
pub struct HostConfig {
    pub relay: String,
    /// The integrated server's port (Open to LAN's).
    pub lan: SocketAddr,
    pub protocol: i32,
    pub game: String,
    pub path: PathMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostState {
    /// Waiting for the relay's answer.
    Registering,
    Sharing(Code),
    /// The relay is unreachable or refused; retrying.
    Retrying(String),
    Stopped,
}

#[derive(Debug)]
pub enum HostEvent {
    /// A new code (the first, or after the relay restarted or wrong codes).
    Code(Code),
    /// A guest's session is carried directly or through the relay.
    Connected {
        session: u64,
        direct: bool,
    },
    Disconnected {
        session: u64,
    },
    /// Someone used the nameplate with the wrong secret.
    WrongCode {
        count: u32,
    },
}

struct Shared {
    config: HostConfig,
    stop: AtomicBool,
    state: Mutex<HostState>,
    events: Mutex<Sender<HostEvent>>,
    /// The current control connection (shut down to stop).
    control: Mutex<Option<TcpStream>>,
    /// Inbound direct streams waiting for their session.
    routes: Mutex<HashMap<u64, (Sender<TcpStream>, Instant)>>,
    sessions: Mutex<usize>,
    wrong: AtomicU32,
}
impl Shared {
    fn emit(&self, e: HostEvent) {
        let _ = self.events.lock().unwrap().send(e);
    }
    fn set(&self, s: HostState) {
        *self.state.lock().unwrap() = s;
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

pub struct Host {
    shared: Arc<Shared>,
    events: Receiver<HostEvent>,
    thread: Option<JoinHandle<()>>,
    accept: Option<JoinHandle<()>>,
}

impl Host {
    /// Start sharing in the background: the code arrives as an event.
    pub fn start(config: HostConfig) -> io::Result<Self> {
        let (tx, events) = channel();
        let listeners = if config.path == PathMode::RelayOnly {
            None
        } else {
            direct::listen(0).ok()
        };
        let shared = Arc::new(Shared {
            config,
            stop: AtomicBool::new(false),
            state: Mutex::new(HostState::Registering),
            events: Mutex::new(tx),
            control: Mutex::new(None),
            routes: Mutex::new(HashMap::new()),
            sessions: Mutex::new(0),
            wrong: AtomicU32::new(0),
        });
        let port = listeners.as_ref().map(|(_, p)| *p);
        let accept = match listeners {
            Some((l, _)) => {
                let s = shared.clone();
                Some(
                    std::thread::Builder::new()
                        .name("horizon-friends-listen".into())
                        .spawn(move || route_inbound(l, s))?,
                )
            }
            None => None,
        };
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("horizon-friends-host".into())
            .spawn(move || run(s, port))?;
        Ok(Self {
            shared,
            events,
            thread: Some(thread),
            accept,
        })
    }
    pub fn state(&self) -> HostState {
        self.shared.state.lock().unwrap().clone()
    }
    pub fn poll(&self) -> Vec<HostEvent> {
        self.events.try_iter().collect()
    }
    /// Guests whose sessions are open (connecting or playing).
    pub fn sessions(&self) -> usize {
        *self.shared.sessions.lock().unwrap()
    }
    /// End the code. Sessions already playing go on until the integrated
    /// server closes them (stopping the LAN server ends them).
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(c) = self.shared.control.lock().unwrap().take() {
            let _ = c.shutdown(Shutdown::Both);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        self.shared.set(HostState::Stopped);
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Hand inbound direct streams to the session they introduce themselves for.
fn route_inbound(listeners: Vec<std::net::TcpListener>, shared: Arc<Shared>) {
    while !shared.stopped() {
        let mut idle = true;
        for l in &listeners {
            let Ok((mut s, _)) = l.accept() else { continue };
            idle = false;
            let shared = shared.clone();
            let _ = std::thread::Builder::new()
                .name("horizon-friends-inbound".into())
                .spawn(move || {
                    let _ = s.set_nonblocking(false);
                    let Ok(session) = direct::read_session(&mut s) else {
                        return;
                    };
                    let route = {
                        let mut routes = shared.routes.lock().unwrap();
                        routes.retain(|_, (_, until)| *until > Instant::now());
                        routes.get(&session).map(|(tx, _)| tx.clone())
                    };
                    if let Some(tx) = route
                        && direct::answer(&mut s, session).is_ok()
                    {
                        let _ = tx.send(s);
                    }
                });
        }
        if idle {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn connect_relay(config: &HostConfig, port: Option<u16>) -> io::Result<(TcpStream, SocketAddr)> {
    let mut last = io::Error::other("no address");
    for relay in resolve_relay(&config.relay)? {
        match direct::connect_from(port, relay, Duration::from_secs(5)) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok((s, relay));
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Register, serve the control connection, register again when it drops
/// (a new code) until stopped.
fn run(shared: Arc<Shared>, port: Option<u16>) {
    let mut backoff = Duration::from_secs(1);
    while !shared.stopped() {
        match register(&shared, port) {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => {
                if shared.stopped() {
                    break;
                }
                shared.set(HostState::Retrying(e.to_string()));
                let until = Instant::now() + backoff;
                while Instant::now() < until && !shared.stopped() {
                    std::thread::sleep(Duration::from_millis(50));
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

fn register(shared: &Arc<Shared>, port: Option<u16>) -> io::Result<()> {
    let config = &shared.config;
    // The control connection uses any port: the direct port stays free for
    // the attach connections, whose observed address is the one that counts.
    let (mut control, relay) = connect_relay(config, None)?;
    control.set_read_timeout(Some(Duration::from_secs(70)))?;
    wire::send(
        &mut control,
        &ToRelay::Host {
            v: wire::VERSION,
            protocol: config.protocol,
            game: config.game.clone(),
        },
    )?;
    let mut reader = BufReader::new(control.try_clone()?);
    let (nameplate, token) = match wire::recv_buffered::<_, FromRelay>(&mut reader)? {
        FromRelay::Registered {
            nameplate, token, ..
        } => (nameplate, token),
        FromRelay::Error { code, .. } => {
            return Err(io::Error::other(format!(
                "the invite service refused: {}",
                code.as_str()
            )));
        }
        other => return Err(io::Error::other(format!("unexpected answer {other:?}"))),
    };
    let code = Code::new(&nameplate)?;
    *shared.control.lock().unwrap() = Some(control.try_clone()?);
    if shared.stopped() {
        let _ = control.shutdown(Shutdown::Both);
        return Ok(());
    }
    shared.wrong.store(0, Ordering::Relaxed);
    shared.set(HostState::Sharing(code.clone()));
    shared.emit(HostEvent::Code(code.clone()));
    loop {
        let message = match wire::recv_buffered::<_, FromRelay>(&mut reader) {
            Ok(m) => m,
            Err(e) => {
                shared.control.lock().unwrap().take();
                return if shared.stopped() || shared.wrong.load(Ordering::Relaxed) >= MAX_WRONG {
                    Ok(())
                } else {
                    Err(e)
                };
            }
        };
        match message {
            FromRelay::Ping => wire::send(&mut control, &ToRelay::Pong)?,
            FromRelay::Join { session } => {
                let mut open = shared.sessions.lock().unwrap();
                if *open >= MAX_SESSIONS {
                    continue;
                }
                *open += 1;
                drop(open);
                let (shared, code, token, control) = (
                    shared.clone(),
                    code.clone(),
                    token.clone(),
                    control.try_clone()?,
                );
                let _ = std::thread::Builder::new()
                    .name("horizon-friends-session".into())
                    .spawn(move || {
                        serve(&shared, &code, &token, session, port, relay, &control);
                        *shared.sessions.lock().unwrap() -= 1;
                    });
            }
            _ => {}
        }
    }
}

/// One guest: attach, agree on the key, settle, connect to the integrated
/// server, carry the session until it ends.
fn serve(
    shared: &Arc<Shared>,
    code: &Code,
    token: &str,
    session: u64,
    port: Option<u16>,
    relay: SocketAddr,
    control: &TcpStream,
) {
    let config = &shared.config;
    let Ok((mut stream, _)) = connect_relay(config, port) else {
        return;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let attach = ToRelay::Attach {
        v: wire::VERSION,
        session,
        token: token.to_owned(),
    };
    if wire::send(&mut stream, &attach).is_err() {
        return;
    }
    let observed = match wire::recv::<_, FromRelay>(&mut stream) {
        Ok(FromRelay::Bridged { observed, .. }) => {
            observed.and_then(|o| o.parse::<SocketAddr>().ok())
        }
        _ => return,
    };
    let Ok(key) = crypto::exchange(&mut stream, &code.secret, &code.nameplate, session) else {
        return;
    };
    let relayed = match session::secure(stream, &key, Role::Host) {
        Ok(s) => s,
        Err(e) => {
            if crypto::is_wrong_code(&e) {
                let count = shared.wrong.fetch_add(1, Ordering::Relaxed) + 1;
                shared.emit(HostEvent::WrongCode { count });
                if count >= MAX_WRONG {
                    // Retire the code: dropping the control connection ends
                    // it at the relay, and `run` registers a new one.
                    let _ = control.shutdown(Shutdown::Both);
                }
            }
            return;
        }
    };
    let candidates = port.map_or_else(Vec::new, |p| direct::candidates(p, Some(relay), observed));
    let shared_routes = shared.clone();
    let settled = session::settle(
        relayed,
        &key,
        Role::Host,
        config.path,
        session,
        port,
        candidates,
        Box::new(move |tx, deadline, _stop| {
            shared_routes
                .routes
                .lock()
                .unwrap()
                .insert(session, (tx, deadline + Duration::from_millis(500)));
        }),
    );
    shared.routes.lock().unwrap().remove(&session);
    let Ok(settled) = settled else {
        return;
    };
    let Ok(server) = TcpStream::connect_timeout(&config.lan, Duration::from_secs(5)) else {
        let (r, _) = settled.secure;
        let _ = r.get_ref().shutdown(Shutdown::Both);
        return;
    };
    let (r, w) = settled.secure;
    shared.emit(HostEvent::Connected {
        session,
        direct: settled.direct,
    });
    if let Ok(t) = crate::pump(server, r, w) {
        let _ = t.join();
    }
    shared.emit(HostEvent::Disconnected { session });
}
