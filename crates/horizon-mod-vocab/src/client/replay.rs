//! Replay playback as a client mod reads and drives it (`replay_state`, `replay_open`...).

/// The replay this window plays, as `replay_state` reads it this frame.
/// Mods receive this snapshot from the game; they do not construct it.
/// `#[non_exhaustive]` is for Rust callers only: on the wire every field and variant is
/// encoded and its layout hashed, so adding one later breaks wasm mods built against this one
/// (docs/modding/src/concepts/wire-types.md).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ReplayState {
    /// The recording's id: its file name in the recordings folder without `.mcpr`, as
    /// `replay_open` takes it. Empty for a recording no id names: one opened from a file
    /// elsewhere (`/studio replay open <path>`), or whose name `replay_open` refuses.
    pub recording: String,
    /// The replay owns the session it plays in, a stand-in for a server that closing the
    /// replay ends, returning to the title screen; it can seek backwards. Every replay is
    /// opened this way today (a live world is left first). False only for a replay played on
    /// top of a session it doesn't own, which only seeks forward.
    pub standalone: bool,
    /// Where the replay's clock is, in ms from the recording's start: from 0 to `duration_ms`.
    pub position_ms: u32,
    /// The recording's length in ms: its recorded duration, or its last packet's time when
    /// that is later.
    pub duration_ms: u32,
    /// The playback speed: 1 is real time, between [`REPLAY_MIN_SPEED`] and [`REPLAY_MAX_SPEED`].
    pub speed: f64,
    /// The clock is paused.
    pub paused: bool,
    /// The clock is at `duration_ms`, every packet played: true whether it got there by playing
    /// or by a seek, false after a seek anywhere before the end.
    pub ended: bool,
}

/// The slowest playback speed `replay_set_speed` takes.
pub const REPLAY_MIN_SPEED: f64 = 0.01;
/// The fastest playback speed `replay_set_speed` takes.
pub const REPLAY_MAX_SPEED: f64 = 20.0;

/// Why a replay call changed nothing, or why an accepted `replay_open` opened nothing
/// (`replay_open_failed`).
/// `#[non_exhaustive]` is for Rust callers only: on the wire every field and variant is
/// encoded and its layout hashed, so adding one later breaks wasm mods built against this one
/// (docs/modding/src/concepts/wire-types.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ReplayError {
    /// Not a recording id: empty, longer than 250 bytes, holding a path separator, a drive,
    /// `..`, a NUL or another control character, ending in a dot or a space, or a name
    /// Windows keeps for a device (`CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`,
    /// in any case, with or without an extension).
    InvalidId,
    /// No recording has this id: no regular file of that name directly in the recordings
    /// folder. A link, or a recordings folder that is itself a link, counts as none.
    NotFound,
    /// The recording is there but can't be played (not a recording, damaged, or of another
    /// version).
    Unreadable,
    /// No replay is open.
    NotOpen,
    /// The speed is not a finite number between [`REPLAY_MIN_SPEED`] and [`REPLAY_MAX_SPEED`].
    InvalidSpeed,
    /// A replay that doesn't own its session can't seek back before where it is.
    BackwardSeek,
    /// This client can't play a replay now: a splitscreen guest or a server, the game is
    /// closing, or every request id has been given. Also why an accepted `replay_open` fails
    /// when its session can't start, the person cancels the wait for it, or the replay playing
    /// keeps its place (its camera path can't be saved, or was just saved into the archive the
    /// request read).
    Unavailable,
    /// A world is being joined, this client is connected to a multiplayer server, or its
    /// single-player world is open to LAN or has splitscreen players: a replay would end that
    /// session. Leave it first (`disconnect`). Also why an accepted
    /// `replay_open` fails when such a session starts before the replay is ready.
    WorldBusy,
    /// An accepted `replay_open` was replaced by a later one while it was still waiting or
    /// being read (`replay_open_failed` only).
    Superseded,
}
