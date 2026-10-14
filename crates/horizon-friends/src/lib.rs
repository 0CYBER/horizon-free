//! Play with friends: a host shares the world it opened to LAN behind an
//! invite code; guests join with the code, directly when the network allows
//! and through the relay otherwise. Both ends carry the game's TCP stream
//! unchanged between loopback ports (the integrated server on the host, a
//! local port the guest's client connects to), end-to-end encrypted with a
//! key only the holders of the code can derive. The design, the protocol and
//! what the relay sees are in docs/play-with-friends.md.
pub mod code;
pub mod crypto;
pub mod direct;
pub mod guest;
pub mod host;
mod session;
pub mod wire;

pub use code::Code;
pub use guest::{JoinError, Tunnel};
pub use host::{Host, HostConfig, HostEvent, HostState};

use std::{
    io,
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    thread::JoinHandle,
};

/// Fill `buf` from the OS random generator.
pub(crate) fn fill_random(buf: &mut [u8]) -> io::Result<()> {
    getrandom::fill(buf).map_err(|e| io::Error::other(e.to_string()))
}

/// Which paths a session may take (`HORIZON_FRIENDS_PATH`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathMode {
    /// Direct when the ends reach each other, else the relay.
    Auto,
    /// Never connect directly: the other end doesn't learn this one's address.
    RelayOnly,
    /// Never fall back to the relay (tests).
    DirectOnly,
}
impl PathMode {
    pub fn from_env() -> Self {
        match std::env::var("HORIZON_FRIENDS_PATH").as_deref() {
            Ok("relay") => Self::RelayOnly,
            Ok("direct") => Self::DirectOnly,
            _ => Self::Auto,
        }
    }
}

/// The relay's address: `HORIZON_RELAY_URL` at run time, else the one
/// compiled in from the same variable. None until a relay is deployed.
pub fn relay_url() -> Option<String> {
    let runtime = std::env::var("HORIZON_RELAY_URL").ok();
    let compiled = option_env!("HORIZON_RELAY_URL").map(str::to_owned);
    runtime
        .into_iter()
        .chain(compiled)
        .map(|s| s.trim().to_owned())
        .find(|s| !s.is_empty())
}

/// `host:port`, optionally written `tcp://host:port`.
pub fn resolve_relay(url: &str) -> io::Result<Vec<SocketAddr>> {
    let address = url
        .trim()
        .trim_start_matches("tcp://")
        .trim_end_matches('/');
    let found: Vec<SocketAddr> = address.to_socket_addrs()?.collect();
    if found.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the relay's name didn't resolve",
        ));
    }
    Ok(found)
}

/// Carry one connection both ways between a plain stream (the game's) and a
/// protected one, until either side ends; then both are closed.
pub fn pump(
    plain: TcpStream,
    mut reader: crypto::SecureReader<TcpStream>,
    mut writer: crypto::SecureWriter<TcpStream>,
) -> io::Result<JoinHandle<()>> {
    use io::{Read, Write};
    for s in [&plain, reader.get_ref(), writer.get_ref()] {
        s.set_read_timeout(None)?;
        let _ = s.set_nodelay(true);
    }
    let mut plain_in = plain.try_clone()?;
    let mut plain_out = plain;
    let outbound = std::thread::Builder::new()
        .name("horizon-friends-out".into())
        .spawn(move || {
            let mut buf = vec![0u8; crypto::MAX_PAYLOAD];
            loop {
                match plain_in.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if writer.write_data(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = writer.get_ref().shutdown(std::net::Shutdown::Both);
            let _ = plain_in.shutdown(std::net::Shutdown::Both);
        })?;
    std::thread::Builder::new()
        .name("horizon-friends-in".into())
        .spawn(move || {
            while let Ok(Some(frame)) = reader.read_frame() {
                if let crypto::Frame::Data(d) = frame
                    && plain_out.write_all(&d).is_err()
                {
                    break;
                }
            }
            let _ = plain_out.shutdown(std::net::Shutdown::Both);
            let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
            let _ = outbound.join();
        })
}
