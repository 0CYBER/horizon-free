//! The protocol pieces the proxy speaks, through the platform's public API (docs/layers.md).

pub use horizon_platform::protocol::{
    Error, PROTOCOL_VERSION,
    buf::{Reader, Writer, write_varint},
    connection::{Closer, ConnReader, ConnWriter, Connection},
    crypto::{offline_uuid, random_bytes, server_hash},
    ids,
    packets::{Empty, handshake, login, status},
    rsa::RsaPrivateKey,
};

/// A translatable component as JSON, for the login and the server list: the client shows it in its own
/// language, and `fallback` where it has no such key (a vanilla client).
pub fn translated_json(key: &str, fallback: &str) -> String {
    serde_json::json!({ "translate": key, "fallback": fallback }).to_string()
}

/// The game version a server list shows.
pub fn version_name() -> &'static str {
    horizon_platform::version::VANILLA_VERSION
}
