//! The `creeper_power` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0191

pub trait Hooks {
    /// Lightning charged a creeper.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn creeper_power();
}
