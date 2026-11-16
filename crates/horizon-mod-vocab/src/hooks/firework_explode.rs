//! The `firework_explode` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0217

pub trait Hooks {
    /// A firework rocket completed its explosion effects.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn firework_explode();
}
