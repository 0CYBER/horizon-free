//! The `item_picked_up_partial` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` picked up part of a ground stack (`ItemEntity.playerTouch`,
    /// right after `Inventory.add` took some of it and left the rest).
    /// `original` is a copy of the stack as it lay before the pickup:
    /// changing it does nothing. `remaining` is what the item entity still
    /// holds, so it starts as the part that did not fit. Set another stack
    /// there to leave the entity holding it, or `None` to empty it: the
    /// pickup then counts `original`'s count less what is left. The amount
    /// taken is `original`'s count less `remaining`'s. A mod that removes the
    /// item entity meanwhile keeps it removed: no take animation, the stat
    /// and criteria still run. Of the two notices, this one hears a partial
    /// pickup and `item_picked_up_post` a whole one; a pickup is never told
    /// to both. Not called when `item_picked_up` refused the pickup or the
    /// inventory took nothing. A creative player always takes the whole
    /// stack. Server-side; no NeoForge counterpart.
    #[notes = r#"## Partial item pickups

- **`item_picked_up_partial(player, original, remaining)`** (Horizon only, no NeoForge counterpart): the inventory took some of a ground stack and left the rest on the ground. `original` is a copy of the stack before the pickup; `remaining` is the item entity's live remainder: set a stack to change what stays, or `None` to empty the entity. The amount taken is `original`'s count less `remaining`'s. A mod that removes the entity keeps it removed, on the host's player and LAN guests alike: no take animation, the stat and criterion still run.
- Which notice hears a pickup: `item_picked_up_post` when the whole stack went in, `item_picked_up_partial` when part of it did, neither when `item_picked_up` refused it or the inventory took nothing. A pickup is told to one of them at most, never both. A creative player always takes the whole stack.
- With no listener the pickup reads one flag and clones nothing."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (Horizon-only; ItemEntityPickupEvent.Post has no partial-pickup notice)"
    )]
    fn item_picked_up_partial();
}
