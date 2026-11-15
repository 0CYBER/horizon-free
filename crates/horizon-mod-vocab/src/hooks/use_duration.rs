//! The `use_duration` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0101

pub trait Hooks {
    /// `Item.getUseDuration` of `stack`, in ticks: `duration` arrives with
    /// vanilla's (a consumable's time, 72000 for a shield, a spear, a bow,
    /// 0 for an item with no use over time). A mod's own item (outside
    /// `minecraft:`) with no `consumable`, `blocks_attacks` or kinetic
    /// weapon that answers above 0 is used over time on a right click, as an
    /// `Item.use` that calls `startUsingItem`, until it is let go or the time
    /// runs out (`living_entity_use_item_finish`). A consumable's time
    /// changes too, potions and the other vanilla items keep their own.
    /// Asked when a use starts, each tick a mod's item is in use, and each
    /// frame the hand draws one in use, on both sides. It has no user, as
    /// it is asked so often: answer from the stack alone.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "none (vanilla Item.getUseDuration; NeoForge adds no event or extension for it); LivingEntityUseItemEvent.Start can set a use's duration"
    )]
    fn use_duration();
}
