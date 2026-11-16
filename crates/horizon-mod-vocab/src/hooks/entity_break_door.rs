//! The `entity_break_door` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0198

pub trait Hooks {
    /// A mob broke the door at `pos`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_break_door();
}
