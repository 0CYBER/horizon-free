//! The `hopper_inventory_search` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0224

pub trait Hooks {
    /// A hopper selected a container while searching for items to push or
    /// pull. Exactly one of `container` and `entity` identifies the target.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's InventoryMoveItemEvent)"
    )]
    fn hopper_inventory_search();
}
