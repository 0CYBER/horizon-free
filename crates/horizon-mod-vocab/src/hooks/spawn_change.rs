//! The `spawn_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0267

pub trait Hooks {
    /// The world's spawn point changed to `pos` and `angle`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn spawn_change();
}
