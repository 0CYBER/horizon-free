//! What the server's console commands with output and player-limit calls return or refuse with.

/// What `Commands::run_command_with_output` gives back: whether the command ran without failing
/// and the console's feedback for it as plain text. A line run where no server is open to
/// players, or empty once its whitespace and one leading slash are dropped, runs nothing and comes
/// back as `ran: false` with no `lines`, as `run_command` answers `false` for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct CommandResult {
    /// Whether the command ran without failing (what `run_command` returns).
    pub ran: bool,
    /// What the console would have printed for it, one entry per line of feedback, as plain text.
    /// At most [`CommandResult::MAX_LINES`] entries of at most [`CommandResult::MAX_LINE_BYTES`]
    /// bytes each; further lines are dropped and a longer one is cut at a character boundary.
    pub lines: Vec<String>,
}

impl CommandResult {
    /// The most feedback lines kept.
    pub const MAX_LINES: usize = 256;
    /// The most bytes kept of one feedback line.
    pub const MAX_LINE_BYTES: usize = 4096;

    /// A result of a command that did or did not run, with its feedback `lines`.
    pub fn new(ran: bool, lines: Vec<String>) -> Self {
        Self { ran, lines }
    }
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
