//! The `on_dropped_by_player` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.onDroppedByPlayer`: `player` (the acting player's
    /// handle) pressed the drop key on `stack`, the whole selected stack
    /// before anything leaves it; `all` is true when the whole stack goes,
    /// false for one item (`ServerPlayer.drop(boolean)`). Set `canceled` and
    /// the stack stays in the slot: nothing is taken out, no item entity is
    /// made, no drop statistic is awarded and `item_toss` does not fire; a
    /// server's player is sent the slot back, undoing their predicted drop.
    /// Not asked for a menu's throw, nor for a spectator (who drops nothing).
    /// Only the `ItemPart`s attached to `stack`'s item are asked: an item
    /// with none costs one table lookup. Server-side, with a read-only `Host`.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.onDroppedByPlayer"
    )]
    fn on_dropped_by_player();
}
