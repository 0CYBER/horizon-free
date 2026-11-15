//! The `difficulty_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0193

pub trait Hooks {
    /// The server's difficulty changed, from `from` to `to` (`peaceful`,
    /// `easy`, `normal`, `hard`; NeoForge's `DifficultyChangeEvent`). It is
    /// vanilla's `MinecraftServer.setDifficulty`, which `/difficulty`, a
    /// dedicated server's management and a test environment's difficulty all
    /// go through, and it is told only when the difficulty is another one. It
    /// only hears: the change has already happened. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "DifficultyChangeEvent"
    )]
    fn difficulty_changed();
}
