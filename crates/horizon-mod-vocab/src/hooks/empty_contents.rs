//! The `empty_contents` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IDispensibleContainerItemExtension.emptyContents`: `stack` (a bucket
    /// of a mod fluid, a jar, a canister) is about to be emptied at `pos`, by
    /// `player` (the acting player) who clicked `hit` (the block and face),
    /// or by a dispenser (`player` and `hit` are `None`). Leave
    /// `answer` `None` for vanilla's emptying. Answer `Some(true)` when the
    /// part placed the contents itself through the host: the stack becomes
    /// an empty `minecraft:bucket` (a creative player keeps it), no vanilla
    /// sound plays and the use counts as a success. Answer `Some(false)` for
    /// a failure: nothing is placed, a player keeps the full stack and a
    /// dispenser drops it, as vanilla's does when emptying fails. Asked for
    /// a player once `bucket_empty` let the pour go on, and for a dispenser
    /// before its own placement; an item with this part reaches the
    /// dispenser's bucket behaviour even when it is no bucket of a mod
    /// fluid, and a `dispense_item` answer still comes first. A server's
    /// client does not predict the pour of an item with this part. The part
    /// is asked in a transaction with the game's apply: a part that changes
    /// the player's hand or the dispenser's slots while answering
    /// `Some(true)` is refused and its journaled writes are undone, as a failed
    /// use (the player keeps the stack, the dispenser fails). A player's
    /// successful pour counts for the `placed_block` criterion. Only the
    /// `ItemPart`s attached to `stack`'s item are asked. Server-side.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "item",
        key = "stack",
        neoforge = "IDispensibleContainerItemExtension.emptyContents"
    )]
    fn empty_contents();
}
