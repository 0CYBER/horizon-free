//! The `potion_splash` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0322

pub trait Hooks {
    /// A thrown splash or lingering potion applied its hit effects.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn potion_splash();
}
