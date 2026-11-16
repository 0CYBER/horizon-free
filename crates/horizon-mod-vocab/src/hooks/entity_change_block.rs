//! The `entity_change_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0200

pub trait Hooks {
    /// A mob changed a block at `pos`, after the new state was applied.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_change_block();
}
