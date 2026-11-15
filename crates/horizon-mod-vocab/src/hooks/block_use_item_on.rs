//! The `block_use_item_on` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockBehaviour.useItemOn`: the block's own behaviour, with the
    /// held item, on the server for each player. An answer replaces the
    /// template's behaviour; `TryWithEmptyHand` tries `useWithoutItem` on
    /// the main hand, and `Pass` goes on to the item's `useOn`.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "block",
        key = "state",
        neoforge = "none (vanilla BlockBehaviour.useItemOn; PlayerInteractEvent.RightClickBlock cancels or answers before it)"
    )]
    fn use_item_on();
}
