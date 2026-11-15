//! `ItemPart::is_primary_item_for` (`IItemExtension.isPrimaryItemFor`): the answers are asked
//! once when parts install and read by every enchantment selection
//! (`horizon_items::primary_items`).
use super::*;

impl Game<'_> {
    /// Ask the `is_primary_item_for` parts of each item once per registered enchantment, with
    /// the item's default stack, and store the answers that differ from vanilla's
    /// `Enchantment.isPrimaryItem` for selection to read: the table's offers and clues, loot
    /// `enchant_with_levels`, trades and spawn gear all choose inside a roll that holds the
    /// level's random, where no part can be asked. Run at world open and on `/reload`, after
    /// the parts install, on the logical server only; elsewhere, or with no such part, the
    /// table is cleared and selection is vanilla's.
    pub(in crate::game) fn resolve_primary_items(&self) {
        let mut answers = horizon_items::primary_items::PrimaryAnswers::new();
        if self.is_logical_server()
            && mod_api::__private::has_part_hook(mod_api::HookId::IsPrimaryItemFor)
        {
            let enchantments = &self.stations().enchantments;
            let items = self.items();
            for item in items.items.keys() {
                if !mod_api::__private::item_listens(mod_api::HookId::IsPrimaryItemFor, item) {
                    continue;
                }
                let stack = ItemStack::new(item, 1);
                for enchantment in &enchantments.list {
                    let vanilla = enchantments.is_primary(items, enchantment, item);
                    let mut primary = vanilla;
                    self.native_is_primary_item_for(&stack, &enchantment.id, &mut primary);
                    if primary != vanilla {
                        answers
                            .entry(item.clone())
                            .or_default()
                            .insert(enchantment.id.clone(), primary);
                    }
                }
            }
        }
        horizon_items::primary_items::install(answers);
    }
}
