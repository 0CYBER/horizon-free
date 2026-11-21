//! The `replay_opened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A replay is open and its first frame is set up: `recording` is its id (as
    /// `replay_open` takes it; empty for a recording no id names), `duration_ms` its length,
    /// `standalone` whether it owns its session, and `request` the id `replay_open` returned
    /// for the mod request that opened it, or 0 for a replay opened any other way (the
    /// replays list, `/studio replay open`, the control socket). Heard for every opening, and
    /// only once the recording has been read: one that can't be played opens nothing and is
    /// not heard of here (a mod's request hears `replay_open_failed`). `host.replay_state()`
    /// reads the replay from here on. A replay opened over another one is heard after
    /// `replay_closed` of that one.
    ///
    /// Its event's fields are fixed: on the wire every field is encoded and its layout hashed,
    /// so adding one later breaks wasm mods built against this one; `#[non_exhaustive]` is for
    /// Rust callers only (docs/modding/src/concepts/wire-types.md).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's replays; NeoForge has no replay playback)"
    )]
    fn replay_opened();
}
