//! Opt-in data collection, shared by the launcher, the game and the ingest server.
//!
//! Nothing is collected or sent until the player says yes, kind by kind ([`Consent`]): the
//! game and the launcher only [`outbox::push`] items whose kind is allowed, and the launcher's
//! [`upload::flush`] drops anything the player has since turned off before it opens a
//! connection. Items are small summaries (percentiles and histogram buckets, never raw streams),
//! dated to the day, with no account, world, server or address in them ([`scrub`]); they leave
//! in zstd batches ([`batch`]). `docs/telemetry.md` lists every field.
//!
//! The one exception is [`live`]: the anonymous "I'm playing" ping the game sends while it
//! runs, for the live player count. It holds no install ID and nothing about the player.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod batch;
pub mod consent;
pub mod live;
pub mod outbox;
pub mod payload;
pub mod scrub;
pub mod system;
#[cfg(feature = "upload")]
pub mod upload;

pub use consent::{Choices, Consent};
pub use payload::{Item, Kind, Payload, Source};

/// The payload format. New optional fields keep it; a field that changes meaning or goes
/// bumps it, and the server keeps reading the older ones.
pub const SCHEMA: u32 = 1;

/// What players agree to. Bumped when what is collected changes: the launcher then asks again,
/// and until the player answers nothing new is kept or sent.
pub const CONSENT_VERSION: u32 = 1;

/// The ingest server, or none while there isn't one: then nothing is ever sent (no ping either)
/// and the outbox stays within its caps. `HORIZON_TELEMETRY_URL` overrides it (tests, a local
/// server).
pub const DEFAULT_ENDPOINT: Option<&str> = None;

/// Where items are sent, if anywhere.
pub fn endpoint() -> Option<String> {
    match std::env::var("HORIZON_TELEMETRY_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url.trim().trim_end_matches('/').to_owned()),
        _ => DEFAULT_ENDPOINT.map(str::to_owned),
    }
}

/// `telemetry/` in the game folder: the consent, the outbox and the upload state, read by the
/// launcher and the game alike.
pub fn dir(home: &Path) -> PathBuf {
    home.join("telemetry")
}

/// Days since 1970-01-01 (UTC): the only time an item carries.
pub fn today() -> u32 {
    (now_secs() / 86_400) as u32
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `bytes` random bytes in lowercase hex.
pub(crate) fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    // Without the OS's randomness the time still makes names that don't collide in practice.
    if getrandom::fill(&mut buf).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (nanos >> (8 * (i % 16))) as u8;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `bytes` through a temporary file, so a reader never sees half of it.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", random_hex(4)));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
pub(crate) mod test_home {
    use std::path::PathBuf;

    /// An empty folder of its own under the system's temporary folder.
    pub fn new(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-telemetry-{name}-{}", super::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
