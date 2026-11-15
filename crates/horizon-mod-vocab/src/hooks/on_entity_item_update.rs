//! The `on_entity_item_update` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.onEntityItemUpdate`: each tick of a dropped item
    /// entity holding `stack`, before the entity's own tick (`ItemEntity.tick`:
    /// its fluids, movement, pickup delay, merging and lifetime), and after
    /// `entity_tick_pre`. `item` is the entity, in the world: read and change
    /// it through the host, its attachments included. Set `canceled` to skip
    /// that tick's vanilla work, as NeoForge's `true` return does; the item
    /// skips movement and its own merge pass; another item can still merge into it.\n    /// `entity_tick_post` still hears it.
    /// Only the `ItemPart`s attached to `stack`'s item hear it: an item with
    /// none costs one table lookup, and a game with no such part none at all.
    /// Server-side, for every dropped item that ticks.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.onEntityItemUpdate"
    )]
    fn on_entity_item_update(stack: &ItemStack, item: ItemEntityRef, canceled: &mut bool);
}
