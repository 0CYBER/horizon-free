//! The `creeper_ignite` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0190

pub trait Hooks {
    /// A player ignited a creeper with an item.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn creeper_ignite();
}
