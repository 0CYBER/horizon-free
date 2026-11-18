//! The server address a player's client typed to connect.

/// The host and port in a client's handshake (`ClientIntentionPacket`): the address as the
/// player typed it, not the IP they connect from.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ConnectAddress {
    /// The host as typed (`play.example.com`, an IP, `localhost`), cut at the first NUL, where a
    /// proxy appends its own data.
    pub host: String,
    /// The port the client said it connects to.
    pub port: u16,
}

impl ConnectAddress {
    /// A host and port.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}
