//! The `replay_closed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The replay of `recording` closed, whatever closed it: a mod's `replay_close`, Escape,
    /// the pause menu's Disconnect, or another replay opening over it. Heard once, after
    /// the replay is gone (`host.replay_state()` is none, or already the next replay's
    /// only after its `replay_opened`) and before a standalone replay's world is left.
    ///
    /// Its event's fields are fixed: on the wire every field is encoded and its layout hashed,
    /// so adding one later breaks wasm mods built against this one; `#[non_exhaustive]` is for
    /// Rust callers only (docs/modding/src/concepts/wire-types.md).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's replays; NeoForge has no replay playback)"
    )]
    fn replay_closed();
}
