use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Registered enchantments, in registry order.
    pub(in crate::game) fn enchantments(&self) -> Vec<EnchantmentInfo> {
        let enchantments = &self.stations().enchantments;
        enchantments
            .list
            .iter()
            .map(|e| {
                mod_api::EnchantmentInfo::new(
                    e.id.to_string(),
                    e.max_level,
                    enchantments.is_curse(&e.id),
                )
            })
            .collect()
    }
    /// `Enchantment.getMinCost(level)`: the enchanting power needed, or None for an unknown id.
    pub(in crate::game) fn enchantment_min_cost(&self, id: &str, level: u32) -> Option<i32> {
        self.stations()
            .enchantments
            .get(id)
            .map(|e| e.min_cost.calculate(level))
    }
    /// `ItemStack.getHoverName()` as plain text, in the client's language.
    pub(in crate::game) fn hover_name(&self, stack: &ItemStack) -> String {
        horizon_client::stations::hover_name(&self.pack.names, stack)
    }
    /// Whether enchantment `id` can go on `item` (`Enchantment.isSupportedItem`).
    pub(in crate::game) fn enchantment_supports(&self, id: &str, item: mod_api::ItemKey) -> bool {
        let Some(item) = mod_api::__private::item_key_id(item) else {
            return false;
        };
        self.stations()
            .enchantments
            .can_enchant(self.items(), id, item)
    }
}
