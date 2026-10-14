//! The guest's end: find the host behind a code, agree on the key, settle
//! on a direct or relayed stream, then offer it on a loopback port the
//! game's client connects to like to any server.
use crate::{
    Code, PathMode,
    crypto::{self, Role},
    direct, resolve_relay,
    session::{self, Secure},
    wire::{self, ErrorCode, FromRelay, ToRelay},
};
use std::{
    fmt, io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    thread::JoinHandle,
    time::Duration,
};

pub struct GuestConfig {
    pub relay: String,
    pub code: Code,
    /// The game's protocol number and version name (the relay refuses a
    /// guest whose protocol differs from the host's).
    pub protocol: i32,
    pub game: String,
    pub path: PathMode,
}

/// Why joining failed, in words for the player.
#[derive(Debug)]
pub enum JoinError {
    /// No relay is configured in this build.
    NotAvailable,
    Unreachable(String),
    UnknownCode,
    Ended,
    VersionMismatch {
        host: Option<String>,
        mine: String,
    },
    HostUnreachable,
    Full,
    RateLimited,
    WrongCode,
    Failed(String),
}
impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAvailable => write!(f, "Joining friends is not available yet in this build."),
            Self::Unreachable(e) => write!(f, "Couldn't reach the invite service ({e})."),
            Self::UnknownCode => write!(
                f,
                "No world uses this code. Check it, or ask your friend for a new one."
            ),
            Self::Ended => write!(
                f,
                "This invite has ended: your friend stopped sharing or left the world."
            ),
            Self::VersionMismatch {
                host: Some(v),
                mine,
            } => {
                write!(f, "Your friend plays Minecraft {v}; you have {mine}.")
            }
            Self::VersionMismatch { host: None, mine } => {
                write!(
                    f,
                    "Your friend plays another version of Minecraft than {mine}."
                )
            }
            Self::HostUnreachable => write!(f, "Your friend's game didn't answer."),
            Self::Full => write!(
                f,
                "Your friend's world is full, or the invite service is busy. Try again in a moment."
            ),
            Self::RateLimited => {
                write!(f, "Too many attempts. Wait a few minutes, then try again.")
            }
            Self::WrongCode => write!(f, "The code doesn't match. Check the last four characters."),
            Self::Failed(e) => write!(f, "Couldn't connect to your friend ({e})."),
        }
    }
}
impl std::error::Error for JoinError {}

/// A settled session, ready to carry the game's connection.
pub struct Tunnel {
    secure: Secure,
    /// Whether the stream goes straight to the host (else through the relay).
    pub direct: bool,
}

/// Find the host, agree on the key and settle on a path. Blocks for up to a
/// few seconds (the direct race).
pub fn join(config: &GuestConfig) -> Result<Tunnel, JoinError> {
    let relays = resolve_relay(&config.relay).map_err(|e| JoinError::Unreachable(e.to_string()))?;
    let direct_allowed = config.path != PathMode::RelayOnly;
    let (listeners, port) = if direct_allowed {
        match direct::listen(0) {
            Ok((l, p)) => (l, Some(p)),
            Err(_) => (Vec::new(), None),
        }
    } else {
        (Vec::new(), None)
    };
    let mut last = io::Error::other("no address");
    let mut stream = None;
    for relay in &relays {
        match direct::connect_from(port, *relay, Duration::from_secs(5)) {
            Ok(s) => {
                stream = Some((s, *relay));
                break;
            }
            Err(e) => last = e,
        }
    }
    let (mut stream, relay) = stream.ok_or_else(|| JoinError::Unreachable(last.to_string()))?;
    let failed = |e: io::Error| JoinError::Failed(e.to_string());
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(failed)?;
    let _ = stream.set_nodelay(true);
    wire::send(
        &mut stream,
        &ToRelay::Join {
            v: wire::VERSION,
            nameplate: config.code.nameplate.clone(),
            protocol: config.protocol,
            game: config.game.clone(),
        },
    )
    .map_err(failed)?;
    let (session, observed) = match wire::recv::<_, FromRelay>(&mut stream).map_err(failed)? {
        FromRelay::Bridged { session, observed } => (session, observed),
        FromRelay::Error { code, game } => {
            return Err(match code {
                ErrorCode::UnknownCode => JoinError::UnknownCode,
                ErrorCode::Ended => JoinError::Ended,
                ErrorCode::VersionMismatch => JoinError::VersionMismatch {
                    host: game,
                    mine: config.game.clone(),
                },
                ErrorCode::HostUnreachable => JoinError::HostUnreachable,
                ErrorCode::Full => JoinError::Full,
                ErrorCode::RateLimited => JoinError::RateLimited,
                ErrorCode::BadRequest => {
                    JoinError::Failed("the invite service refused the request".into())
                }
            });
        }
        other => return Err(JoinError::Failed(format!("unexpected answer {other:?}"))),
    };
    let key = crypto::exchange(
        &mut stream,
        &config.code.secret,
        &config.code.nameplate,
        session,
    )
    .map_err(|_| JoinError::HostUnreachable)?;
    let relayed = session::secure(stream, &key, Role::Guest).map_err(|e| {
        if crypto::is_wrong_code(&e) {
            JoinError::WrongCode
        } else {
            JoinError::HostUnreachable
        }
    })?;
    let observed = observed.and_then(|o| o.parse::<SocketAddr>().ok());
    let candidates = port.map_or_else(Vec::new, |p| direct::candidates(p, Some(relay), observed));
    let settled = session::settle(
        relayed,
        &key,
        Role::Guest,
        config.path,
        session,
        port,
        candidates,
        Box::new(move |tx, deadline, stop| direct::accept(listeners, session, deadline, stop, tx)),
    )
    .map_err(|e| JoinError::Failed(e.to_string()))?;
    Ok(Tunnel {
        secure: settled.secure,
        direct: settled.direct,
    })
}

impl Tunnel {
    /// Offer the session on `127.0.0.1:port` for one connection (the game's
    /// client); the returned thread ends when that connection does.
    pub fn serve_loopback(self) -> io::Result<(u16, JoinHandle<()>)> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let thread = std::thread::Builder::new()
            .name("horizon-friends-guest".into())
            .spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                let plain = loop {
                    match listener.accept() {
                        Ok((s, from)) if from.ip().is_loopback() => break Some(s),
                        Ok(_) => {}
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() > deadline {
                                break None;
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break None,
                    }
                };
                drop(listener);
                let (reader, writer) = self.secure;
                let Some(plain) = plain else {
                    let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
                    return;
                };
                let _ = plain.set_nonblocking(false);
                if let Ok(t) = crate::pump(plain, reader, writer) {
                    let _ = t.join();
                }
            })?;
        Ok((port, thread))
    }
    /// The protected stream itself (tests, or a client that reads it
    /// directly).
    pub fn into_stream(
        self,
    ) -> (
        crypto::SecureReader<TcpStream>,
        crypto::SecureWriter<TcpStream>,
    ) {
        self.secure
    }
}
