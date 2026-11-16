//! The `on_item_use_first` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.onItemUseFirst`: the held item's first say on a
    /// click on a block, on the server for each player, after
    /// `use_item_on_block` (`ItemBeforeBlock`) let it through and before the
    /// block's own `useItemOn`. An answer ends the click: neither the block
    /// nor the item's `useOn` runs. `Pass` goes on to the block.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.onItemUseFirst"
    )]
    fn on_item_use_first();
}
