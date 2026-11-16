//! The `player_swap_with_equipment_slot` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0314

pub trait Hooks {
    /// A server player swapped their main and off hands, after both inventory slots changed.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_swap_with_equipment_slot();
}
