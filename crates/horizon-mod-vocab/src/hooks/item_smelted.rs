//! The `item_smelted` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `FurnaceResultSlot.checkTakeAchievements` ran for a server player at a
    /// furnace, blast furnace or smoker's result slot: `player` is their UUID
    /// and `player_entity_id` their network id; `stack` and `amount_removed`
    /// are what vanilla passes there (`getSmelting`, `getAmountRemoved`). A
    /// click or a throw passes the taken stack and its count; a shift-click
    /// passes the slot's stack as it was with the count moved, then what
    /// stayed in the slot (`None` once it is empty) with 0; a hotbar swap
    /// passes the taken stack with 0. Fires before `furnace_extract`.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.ItemSmeltedEvent",
        cancellable = false
    )]
    fn item_smelted();
}
