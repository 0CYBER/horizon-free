//! The `level_load` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0138

pub trait Hooks {
    /// `dimension`'s level opened on the server (NeoForge's `LevelEvent.Load`,
    /// posted in `MinecraftServer.createLevels`): before any of its chunks
    /// (`chunk_load`), so a mod sets up what it keeps for the level
    /// here. The level the world opens in comes before `server_started`; the
    /// others open when someone first goes there (a player, a portal, an
    /// `execute in` command), with the level in place. Once for each open:
    /// `level_unload` follows. Explicitly opened runtime levels also fire this while
    /// parked: `event.dimension` names the opened level, not the Host's current level.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Load",
        cancellable = false
    )]
    fn level_load();
}
