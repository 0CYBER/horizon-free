//! The `replay_advanced` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A once-a-frame sample of the replay's clock, on a frame it moved: `position_ms` is where
    /// it is now, in ms from the recording's start (0 to its length), after the recording's
    /// packets up to there were processed in order (a packet the game cannot decode or apply
    /// is skipped). Not heard on a frame the clock stood still (paused,
    /// at the end, or the frame it opened on, which `replay_opened` tells of). After a seek,
    /// the next one is heard on the frame the seek is first drawn, with exactly the position
    /// the seek applied, even when that is where the clock already was; the clock runs on from
    /// there on the frame after. For a timeline or a camera path that follows playback.
    ///
    /// Its event's fields are fixed: on the wire every field is encoded and its layout hashed,
    /// so adding one later breaks wasm mods built against this one; `#[non_exhaustive]` is for
    /// Rust callers only (docs/modding/src/concepts/wire-types.md).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's replays; NeoForge has no replay playback)"
    )]
    fn replay_advanced();
}
