//! The `living_entity_use_item_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0132

pub trait Hooks {
    /// NeoForge's `LivingEntityUseItemEvent.Tick`: each tick `entity` uses
    /// `stack` in `hand`, where `LivingEntity.updatingUsingItem` starts,
    /// `duration` the ticks left. `entity` is as in
    /// `living_entity_use_item_start`. Change `duration` to move the
    /// end (consumables, potions, spyglasses and mods' items count to it,
    /// and so do a client's eating crumbs and first person pose, as the
    /// started duration of `living_entity_use_item_start` does). A
    /// cancel sets it to -1. Once a mod set it, the use goes on
    /// only while it is above 0: at 0 or less, by a cancel or set, the
    /// item's tick is skipped and the server finishes the use
    /// (`completeUsingItem`: `finishUsingItem`, the food eaten,
    /// `living_entity_use_item_finish`), while an item used on
    /// release (a crossbow) stays frozen until let go. NeoForge's javadoc
    /// says a cancel stops the use, but its code finishes it; this follows
    /// the code. On a server's client a run-out use stays in use, frozen,
    /// so letting go still tells the server. Both sides, as vanilla's
    /// `updatingUsingItem`: answer the same on both, and change the world
    /// only on the server (`native_window` is false there), not on a
    /// server's client. Runs every tick of every use:
    /// check the item first.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "LivingEntityUseItemEvent.Tick",
        cancellable = true
    )]
    fn living_entity_use_item_tick();
}
