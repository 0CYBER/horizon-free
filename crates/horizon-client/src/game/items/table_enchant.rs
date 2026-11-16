//! The enchanting table's result (`EnchantmentMenu.clickMenuButton`), with
//! `ItemPart::apply_enchantments` (`IItemExtension.applyEnchantments`) asked for it.
use super::*;

/// Says once per item that its `apply_enchantments` parts answered a count other than the
/// input's, which the table can't hold.
fn refuse_table_count(item: &str, input: u32, answered: u32) {
    static REFUSED: std::sync::Mutex<std::collections::BTreeSet<String>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    if REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(item.to_owned())
    {
        log::warn!(
            "Horizon: an apply_enchantments part of {item} answered a count of {answered} for an input of {input}; the table's own result is used"
        );
    }
}

impl Game<'_> {
    /// The stack the table's `row`, costing `cost`, makes of `input` at `seed`: vanilla's
    /// (`horizon_client::stations::apply_enchantment`), then the parts of the input's item
    /// (`ItemPart::apply_enchantments`) with the list it chose. `None` when the row gives
    /// nothing. A result whose count isn't the input's is refused for vanilla's. An item with
    /// no such part costs one table lookup.
    pub(in crate::game) fn table_enchant(
        &self,
        seed: i32,
        row: usize,
        cost: i32,
        input: &ItemStack,
    ) -> Option<ItemStack> {
        use horizon_client::stations;
        let enchantments = &self.stations().enchantments;
        if !mod_api::__private::item_listens(mod_api::HookId::ApplyEnchantments, &input.item) {
            return stations::apply_enchantment(enchantments, self.items(), seed, row, cost, input);
        }
        let list = stations::enchantment_list(enchantments, self.items(), seed, row, cost, input);
        let vanilla = stations::apply_enchantment_list(input, list.clone())?;
        let mut result = vanilla.clone();
        self.native_apply_enchantments(input, &list, &mut result);
        if result.count != input.count {
            refuse_table_count(&input.item, input.count, result.count);
            return Some(vanilla);
        }
        Some(result)
    }
}
