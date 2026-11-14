//! Why the server's console and player-limit calls refuse.

/// Why `server_run_command_captured` ran nothing. Nothing was run when one comes back; a command
/// that ran and failed is `Ok((false, lines))`, not a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ConsoleRefusal {
    /// No server is open to players here (a client, or a server nobody can join).
    NotOpen,
    /// The line is empty once its whitespace and one leading slash are dropped.
    Blank,
}

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
