//! The `on_use_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0133

pub trait Hooks {
    /// `Item.onUseTick` of `stack`, used by `entity` with `remaining` ticks
    /// left, always above 0: what a mod's own item does each tick it is used
    /// (cook, charge, drain). It comes where `ItemStack.onUseTick` calls the
    /// item, once `living_entity_use_item_tick` let the use go on:
    /// after a consumable's crumbs, for consumables, shields and mods' items
    /// (vanilla's other items do their own, and a kinetic weapon has none on
    /// the server). Both sides, as `living_entity_use_item_tick`:
    /// change the world only where this `Game` runs the world, not on a
    /// server's client. Runs every tick of every use: check the item first.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "LivingEntityUseItemEvent.Tick"
    )]
    fn on_use_tick();
}
