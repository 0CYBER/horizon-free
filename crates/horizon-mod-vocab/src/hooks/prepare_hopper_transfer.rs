//! The `prepare_hopper_transfer` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0323

pub trait Hooks {
    /// A hopper at `pos` began an item transfer attempt. This runs after its
    /// cooldown and enabled checks, before pushing or pulling any items.
    /// Server-side, including the single-player server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no hopper event)"
    )]
    fn prepare_hopper_transfer();
}
