//! The `item_tint_source` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Client-side colour of an item tint source a mod registered (`register_item_tint_sources`,
    /// vanilla's `ItemTintSource.calculate`), asked for each entry on it each time the item is
    /// resolved, in hand, in a screen or on the ground. `config` borrows the entry converted once to a `Tag` at load from the JSON object in the
    /// model's `tints`, so a source reads its own fields there. Set color, ARGB with an alpha of
    /// `0xFF` as vanilla's sources give, only for a source your mod owns; left alone, the entry draws
    /// white. The owned stack snapshot is the one being resolved, including nested bundle items.
    /// `bundle_selected` is the runtime bundle selection index; -1 means none.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla ItemTintSource.calculate; RegisterColorHandlersEvent.ItemTintSources only registers the source)"
    )]
    fn item_tint_source();
}
