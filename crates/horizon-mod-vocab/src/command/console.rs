//! The server console as a mod writes to it: the level of a line and why a line is refused.

/// The level of a line a mod writes to the server console (`Logger.info`, `warning`, `severe`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ConsoleLevel {
    /// `INFO`.
    Info,
    /// `WARN`.
    Warn,
    /// `ERROR`.
    Error,
}

/// Why `host.messages().console_log` wrote nothing. The line is then not logged at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ConsoleLogRefusal {
    /// The mod id is empty, so the line would have no prefix to tell its mod by.
    EmptyModId,
    /// The mod id is longer than 64 bytes.
    ModIdTooLong,
    /// The mod id holds a control character (a line break would forge a second log line).
    InvalidModId,
}

impl crate::NoBackend for ConsoleLogRefusal {
    fn no_backend() -> Self {
        Self::EmptyModId
    }
}
