//! The `can_place_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether `stack` may be placed into `slot` of the mod container block entity `entity`
    /// at `pos` (vanilla's `Container.canPlaceItem`), checked by every shared container
    /// insertion path: hoppers, droppers, `/loot insert`, and the `ITEMS` capability
    /// (`is_valid`, `capacity`, `insert`, `insert_kind`). Asked on the authoritative server
    /// only for slots listed in the container's `ModContainer::inputs`. Return `Some(false)`
    /// to refuse `stack` in `slot`, `Some(true)` to accept it, or `None` to keep the
    /// container's default admission rule (accepted for every slot in `inputs`). Only the
    /// block entity's own attached [`BlockEntityPart`] is asked; containers without a
    /// subscribed part pay only a single dispatch guard.
    #[hook(
        side = "server",
        contract = "replace",
        out = "allowed",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla Container.canPlaceItem; NeoForge's IItemHandler.isItemValid is the capability counterpart)"
    )]
    fn can_place_item();
}
