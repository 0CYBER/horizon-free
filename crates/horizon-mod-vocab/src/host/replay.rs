//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `replay` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ReplayClientHost {
    /// Side: client.
    /// The replay this window plays, if any, as its clock stands now; none while no replay
    /// plays. While one asked for by `replay_open` is still opening, this is the replay
    /// playing, if any.
    #[access(ClientSetup)]
    fn replay_state(&self) -> Option<ReplayState>;
    /// Side: client.
    /// Asks to open the recording `recording` for playback. The id is the recording's file
    /// name in the recordings folder without `.mcpr`, never a path: what an id may hold is in
    /// [`ReplayError::InvalidId`], and an id naming no regular file directly in the folder is
    /// `NotFound`. The call opens the file once, through the folder it checked, and checks it
    /// starts with a zip signature (`Unreadable` if not); the file is then read and indexed
    /// off the game's thread, never reopened by name, and an archive found broken there fails
    /// through `replay_open_failed` with `Unreadable`.
    ///
    /// `Ok(request)` means the request is accepted: `request` is a non-zero id, never given
    /// twice while the game runs (once every `u32` has been given, `replay_open` is
    /// `Unavailable`). Exactly one of `replay_opened` or `replay_open_failed` follows with that
    /// id, whatever happens, except when the game closes first (then no event is promised). A
    /// later `replay_open` replaces a request still waiting or being read, which then fails
    /// with `Superseded`; once a read request waits for a world to be left, a later one is
    /// `WorldBusy` instead. An open asked while a replay plays fails with
    /// `Unavailable` when the camera path edited in that replay can't be saved first, or when
    /// it reads the very archive the editor saved into since the request (its camera path or a
    /// marker); the replay playing stays.
    ///
    /// A replay is never played over a live session it would end: from a plain single-player
    /// world, the world is saved and left as Save and Quit does once the recording is ready,
    /// and the replay opens on the title; a replay already playing is closed first
    /// (`replay_closed`). `WorldBusy` while a world is being joined, a multiplayer server is
    /// connected, or the single-player world is open to LAN or has splitscreen players; an
    /// accepted request whose session changes before it is ready (a world joined, left or
    /// opened, or the replay playing closed or replaced) fails with `WorldBusy` too.
    /// `Unavailable` for a splitscreen guest, a server, or while the game is closing.
    #[access(WorldOnly)]
    fn replay_open(&mut self, recording: &str) -> Result<u32, ReplayError>;
    /// Side: client.
    /// Closes the replay at the end of this frame, as Escape does: a replay opened from the
    /// title screen goes back there. `replay_closed` tells when it is closed. A later
    /// `replay_close` or `replay_seek` in the same frame replaces this request, and a request
    /// for a replay closed or replaced since is dropped.
    /// `NotOpen` while no replay is open.
    #[access(WorldOnly)]
    fn replay_close(&mut self) -> Result<(), ReplayError>;
    /// Side: client.
    /// Pauses the replay's clock now; pausing a paused replay changes nothing. `NotOpen` while
    /// no replay is open.
    #[access(WorldOnly)]
    fn replay_pause(&mut self) -> Result<(), ReplayError>;
    /// Side: client.
    /// Starts the replay's clock again now. `NotOpen` while no replay is open.
    #[access(WorldOnly)]
    fn replay_resume(&mut self) -> Result<(), ReplayError>;
    /// Side: client.
    /// Sets the playback speed now: 1 is real time, 0.5 half and 2 twice as fast.
    /// `InvalidSpeed`, changing nothing, for a speed that is not finite or is outside
    /// [`REPLAY_MIN_SPEED`]..=[`REPLAY_MAX_SPEED`]; `NotOpen` while no replay is open.
    #[access(WorldOnly)]
    fn replay_set_speed(&mut self, speed: f64) -> Result<(), ReplayError>;
    /// Side: client.
    /// Moves the replay's clock to `ms` from the recording's start (at most its length) at
    /// the end of this frame. Then the recording's packets up to the position the seek applied
    /// have been processed in order (a packet the game cannot decode or apply is skipped), and
    /// the next `replay_advanced` reports exactly that position, on the frame drawn with it; the clock runs on from there only on the frame
    /// after. A standalone replay sought back starts again from a fresh session; what the game
    /// keeps outside that session may carry over (today the difficulty shown, and the backing
    /// state of a screen a packet opened). State that came only from the client's own
    /// simulation between packets (an entity moving between its updates, particles) is not
    /// promised either way. How the game gets there, and how long it takes, is not part of this
    /// contract: today a seek forward costs the packets between the clock and the target,
    /// while a seek back in a standalone replay replays every packet from the start, as does a
    /// seek forward across a relative teleport of the player or one whose read fails. When a
    /// rebuild fails, or the forward read fails in a replay that doesn't own its session, the
    /// replay closes (`replay_closed`) and no `replay_advanced` reports the seek.
    /// A standalone replay seeks either way; one that doesn't own its session only forward
    /// (`BackwardSeek`). A later `replay_close` or `replay_seek` in the same frame
    /// replaces this request, and a request for a replay closed or replaced since is
    /// dropped. `NotOpen` while no replay is open.
    #[access(WorldOnly)]
    fn replay_seek(&mut self, ms: u32) -> Result<(), ReplayError>;
}
