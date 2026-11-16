//! Stack-aware remainder decisions made before borrowing the brewing stand for consumption.
use super::*;
use horizon_client::items::Items;

pub(super) struct BrewingRemainder(Option<Option<ItemStack>>);

impl BrewingRemainder {
    pub(super) fn resolve(&self, items: &Items, stack: &ItemStack) -> Option<ItemStack> {
        self.0
            .clone()
            .unwrap_or_else(|| crate::crafting::CraftingRemainder::crafting_remainder(items, stack))
    }
}

impl Game<'_> {
    pub(super) fn brewing_fuel_remainder(&self, pos: (i32, i32, i32)) -> BrewingRemainder {
        let answer = match self.world.block_entities.get(&pos) {
            Some(BlockEntity::Brewing(b)) => b.items[4]
                .as_ref()
                .filter(|s| {
                    b.fuel <= 0
                        && mod_api::__private::item_listens(
                            mod_api::HookId::CraftingRemainder,
                            &s.item,
                        )
                        && horizon_client::brewing::brewing_fuel(world_items!(self), s).is_some()
                })
                .map(|s| self.crafting_remainder(s)),
            _ => None,
        };
        BrewingRemainder(answer)
    }

    pub(super) fn brewing_ingredient_remainder(
        &self,
        pos: (i32, i32, i32),
        cancelled: bool,
    ) -> Option<ItemStack> {
        if cancelled {
            return None;
        }
        match self.world.block_entities.get(&pos) {
            Some(BlockEntity::Brewing(b)) => {
                b.items[3].as_ref().and_then(|s| self.crafting_remainder(s))
            }
            _ => None,
        }
    }
}
