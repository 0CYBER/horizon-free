//! `ItemPart::is_primary_item_for` (`IItemExtension.isPrimaryItemFor`): the logical server asks
//! the answers once when its parts install and stores them on the enchantment registry snapshot
//! it plays with, which every enchantment selection reads (`horizon_items::primary_items`).
use super::*;

impl Game<'_> {
    /// Ask the `is_primary_item_for` parts of each item once per registered enchantment, with
    /// the item's default stack, and store the answers that differ from vanilla's
    /// `Enchantment.isPrimaryItem` on this server's enchantment snapshot for selection to read:
    /// the table's offers and clues, loot `enchant_with_levels`, trades and spawn gear all
    /// choose inside a roll that holds the level's random, where no part can be asked. Run at
    /// world open and on `/reload`, after the parts install, and only on the logical server: a
    /// client's tag install never asks, writes or clears them, so on an integrated server the
    /// client's refresh leaves the server's answers alone. With no such part the snapshot's
    /// answers are retired and selection is vanilla's.
    pub(in crate::game) fn resolve_primary_items(&self) {
        if !self.is_logical_server() {
            return;
        }
        let mut answers = horizon_items::primary_items::PrimaryAnswers::new();
        if mod_api::__private::has_part_hook(mod_api::HookId::IsPrimaryItemFor) {
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
        for table in self.primary_answer_tables() {
            table.install(answers.clone());
        }
    }
    /// Leave the world's parts as its world closes (`clear_game_parts`), with what was resolved
    /// from them: this server's `is_primary_item_for` answers are retired. A client holds none.
    pub(in crate::game) fn clear_world_parts(&self) {
        if self.is_logical_server() {
            for table in self.primary_answer_tables() {
                table.clear();
            }
        }
        mod_api::__private::clear_game_parts();
    }
    /// The answer tables of the enchantment snapshots this game selects with: its stations'
    /// and its items' (loot), once when they are one registry.
    fn primary_answer_tables(
        &self,
    ) -> impl Iterator<Item = &horizon_items::primary_items::PrimaryAnswerTable> {
        let stations = &self.stations().enchantments;
        let items = &self.items().enchantments;
        let other = (!std::sync::Arc::ptr_eq(stations, items)).then_some(items);
        std::iter::once(stations)
            .chain(other)
            .map(|enchantments| enchantments.primary_answers())
    }
}
