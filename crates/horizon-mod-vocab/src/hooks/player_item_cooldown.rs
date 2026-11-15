//! The `player_item_cooldown` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0290

pub trait Hooks {
    /// An item cooldown is about to start for a server player. `item` is the item id and `ticks` its
    /// duration. The player's state is not swapped in. Set `cancel` to skip starting the cooldown.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Paper's PlayerItemCooldownEvent)",
        cancellable = true
    )]
    fn player_item_cooldown();
}
