//! What the server's console commands with output and player-limit calls return or refuse with.

/// Why `server_set_max_players` changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PlayerLimitRefusal {
    /// No server is open to players here (a client, or a server nobody can join).
    NotOpen,
    /// The limit is above `i32::MAX`, vanilla's `max-players` is an `int`.
    TooHigh {
        /// The highest limit taken, `i32::MAX`.
        maximum: u32,
    },
}
