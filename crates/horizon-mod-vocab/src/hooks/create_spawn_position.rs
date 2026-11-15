//! The `create_spawn_position` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a new server level selects its initial world spawn
    /// (NeoForge's `LevelEvent.CreateSpawnPosition`).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LevelEvent.CreateSpawnPosition",
        cancellable = true
    )]
    fn create_spawn_position();
}
