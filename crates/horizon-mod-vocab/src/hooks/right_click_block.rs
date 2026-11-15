//! The `right_click_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0039

pub trait Hooks {
    /// A player right clicks a block (NeoForge's
    /// `PlayerInteractEvent.RightClickBlock`): once per hand, where
    /// `useItemOn` starts, on the clicking client and on the server (`click`
    /// says which, with the block, the hit, the hand and its stack, and the
    /// player), for a spectator too (the server then only lets one open a
    /// menu). Before the block's own use (a chest, a door, a lever) and the
    /// item's (`Item.useOn`, `native_use_on`).
    ///
    /// `use_block`: False skips; True overrides sneak; Default skips when sneaking with either hand full
    /// (sneaking still blocks use when both stacks bypass sneak use). `use_item`: False skips; True forces
    /// even an empty hand or cooldown; Default requires a held stack off cooldown. Empty hands reach
    /// ItemAfterBlock with no stack, but no item Part. Adventure restrictions still apply; block consumption ends the click. Cancelling
    /// skips both and answers `cancellation_result` in their place:
    /// `Pass` (the default) goes on to the item's own use (`Item.use`) and
    /// then the other hand, `Fail` ends the click, and `Success`,
    /// `SuccessServer` or `Consume` end it as used, with the swing they name.
    /// A spectator never reaches item use after a `Pass`; the client goes
    /// straight to the other hand instead.
    /// A mod that keeps players out of land cancels with `Fail`.
    /// On the server, a consuming result fires `any_block_use` once, including
    /// a consuming cancel. The inner criteria are skipped by cancellation unless
    /// the mod explicitly triggers them; the helpers fire only their named criterion.
    /// A consuming item phase fires `item_used_on_block` with the pre-block
    /// snapshot (including an empty hand) before the outer criterion.
    /// The outer criterion sees the stack held when block use started, with the
    /// action's own in-place changes: consumption, damage, component edits, and
    /// the creative-mode count restore. It sees an empty stack if the action
    /// used it up. Neither a listener's hand replacement nor the action's whole
    /// replacement (such as a bucket becoming a water bucket) changes this original.
    /// This matches vanilla, which passes the same stack object it read before
    /// the action. Single player uses this same server contract.
    /// Item-internal triggers (such as shears) can legitimately
    /// fire `item_used_on_block` in addition to the game mode's trigger.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerInteractEvent.RightClickBlock",
        cancellable = true
    )]
    fn right_click_block();
}
