//! The `item_picked_up_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` picked up the whole stack of the ground item entity `item`
    /// (`ItemEntity.playerTouch`, right after `Inventory.add` took all of
    /// it), the entity `item_picked_up` named. `original` is a copy of
    /// the stack as it lay before the pickup: changing it does nothing.
    /// `remaining` is what the item entity still holds, so it starts as
    /// `None` (empty). Set a stack there to leave the entity on the ground
    /// holding it: the pickup then counts only `original`'s count less its
    /// count. Neither says how many were picked up; that is `original`'s
    /// count less `remaining`'s. A mod that removes the item entity meanwhile
    /// keeps it removed: no take animation, the stat and criteria still run.
    /// Not called on a partial pickup, where some of the stack went in and
    /// the rest stayed on the ground: `item_picked_up_partial` hears that one,
    /// and a pickup is never told to both. Nor when `item_picked_up` refused
    /// the pickup or the inventory took nothing; a forced `True` from
    /// `item_picked_up` changes none of this. A creative player always takes
    /// the whole stack. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ItemEntityPickupEvent.Post",
        version = 2
    )]
    fn item_picked_up_post();
}
