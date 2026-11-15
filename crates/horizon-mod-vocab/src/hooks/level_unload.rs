//! The `level_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0139

pub trait Hooks {
    /// `dimension`'s level closed (NeoForge's `LevelEvent.Unload`, posted after
    /// `level.close()` in `MinecraftServer.stopServer`), after its last save:
    /// drop what the mod kept for it. A level other than the Overworld closes
    /// once nobody has been in it for 300 ticks, after its chunks' unloads;
    /// every level still open closes when the server stops, before
    /// `WorldCloseFinal`, and when its open or its loop fails, before
    /// `WorldOpenFailed` or `WorldCloseFailed`. Once for each
    /// `level_load`. Don't read its blocks. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Unload",
        cancellable = false
    )]
    fn level_unload();
}
