//! The `entity_breed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0199

pub trait Hooks {
    /// Two mobs completed a breeding action. `child` is absent when vanilla
    /// lays an egg instead of creating a baby.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_breed();
}
