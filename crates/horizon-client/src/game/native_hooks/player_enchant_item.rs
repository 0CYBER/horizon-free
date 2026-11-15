use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player enchanted `item` at an enchanting table
    /// (`EnchantmentMenu.clickMenuButton`): `enchantments` are the ones it was
    /// given, in order, as (id, level). `item` is the enchanted stack (an
    /// enchanted book for a book). Fires after the levels are paid, before
    /// `enchant_item`. Server-side.
    pub(in crate::game) fn native_player_enchant_item(
        &mut self,
        player: u128,
        player_entity_id: i32,
        item: &ItemStack,
        enchantments: &[(std::sync::Arc<str>, u32)],
    ) {
        self.fire_player_enchant_item(player, player_entity_id, item, enchantments);
    }
}
