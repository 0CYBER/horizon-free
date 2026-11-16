//! The `inventory_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0052

pub trait Hooks {
    /// A server tick of `stack`, the stack a player carries in `slot`: where
    /// `Inventory.tick` calls `Item.inventoryTick`, for every stack, each tick,
    /// `/tick freeze` included (it stops the world, not the player). `slot` is
    /// as `Inventory` saves it (0 to 35; the armour 100 feet to 103 head; -106
    /// the off hand), and `selected` says it is the held slot.
    /// `ItemPlace::PlayerSlot { player, slot }` reads and changes the stack,
    /// which stays in its slot: an emptied one empties the slot, and the
    /// player's client gets the change. Only the `ItemPart`s attached to
    /// `stack`'s item hear it. Server-side, for every living player of the world (the helpers
    /// read the acting one).
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.inventoryTick; NeoForge adds no event or extension for it); PlayerTickEvent ticks the player, not each stack"
    )]
    fn inventory_tick();
}
