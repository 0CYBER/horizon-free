use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// An enchanting table at `pos` drew the level of one offer
    /// (`EnchantmentMenu.slotsChanged`, after `getEnchantmentCost`): `offer`
    /// holds the server player whose table it is, the row (0 to 2), the
    /// power (bookshelves counted), the item and the level drawn. Change
    /// `level` to change the offer; a level below `row + 1` offers nothing,
    /// as vanilla's. Asked for each row in order, before
    /// `prepare_item_enchant`. Server-side.
    pub(in crate::game) fn native_enchantment_level_set(
        &mut self,
        pos: [i32; 3],
        offer: &mod_api::item::stations::EnchantmentRow<'_>,
        level: &mut i32,
    ) {
        self.fire_enchantment_level_set(pos, offer, level);
    }
}
