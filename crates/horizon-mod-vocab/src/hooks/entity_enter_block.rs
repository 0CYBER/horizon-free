//! The `entity_enter_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0202

pub trait Hooks {
    /// A bee entered the hive at `pos` and was stored there.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_enter_block();
}
