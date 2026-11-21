//! The `replay_open_failed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A `replay_open` the game accepted opened nothing: `request` is the id the call
    /// returned, `recording` the id it was asked for, `error` why:
    /// - `Superseded`: a later `replay_open` replaced it before the game took up its read
    ///   recording;
    /// - `Unreadable`: the recording turned out damaged or of another version once read;
    /// - `WorldBusy`: a world was joined, left or opened, or the replay playing closed or
    ///   replaced, before it was ready, or another world entry took its place while it waited;
    /// - `Unavailable`: the replay's session couldn't start, the game began closing, the
    ///   person cancelled the wait for it (Escape while the world loads), or the replay playing
    ///   kept its place: the camera path edited in it couldn't be saved, or the editor saved
    ///   into the very archive this request read (which holds it as it was before).
    ///
    /// Every accepted request ends in exactly one `replay_opened` or `replay_open_failed` with
    /// its id; only a game closing first promises neither. A single-player world already left
    /// for the replay stays left; anything else stays as it was.
    ///
    /// Its event's fields are fixed: on the wire every field is encoded and its layout hashed,
    /// so adding one later breaks wasm mods built against this one; `#[non_exhaustive]` is for
    /// Rust callers only (docs/modding/src/concepts/wire-types.md).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's replays; NeoForge has no replay playback)"
    )]
    fn replay_open_failed();
}
