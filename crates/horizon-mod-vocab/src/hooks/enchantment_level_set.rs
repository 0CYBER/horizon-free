//! The `enchantment_level_set` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An enchanting table at `pos` drew the level of one offer
    /// (`EnchantmentMenu.slotsChanged`, after `getEnchantmentCost`): `offer`
    /// holds the server player whose table it is, the row (0 to 2), the
    /// power (bookshelves counted), the item and the level drawn. Change
    /// `level` to change the offer; a level below `row + 1` offers nothing,
    /// as vanilla's. Asked for each row in order, before
    /// `prepare_item_enchant`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EnchantmentLevelSetEvent",
        cancellable = false
    )]
    fn enchantment_level_set();
}
