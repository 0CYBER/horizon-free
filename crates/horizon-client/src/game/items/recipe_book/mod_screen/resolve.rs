//! Resource-backed resolution of mod recipe displays.
use crate::game::{Game, api_values::TryIntoEngine};
use horizon_client::{crafting::Ingredient, items::ItemStack, menu::place::Stacked};
use mod_api::{RecipeBookDisplay as Display, RecipeSlotDisplay as Slot};

pub(super) struct Resolver<'a, 'w> {
    game: &'a Game<'w>,
    stocks: &'a Stacked,
    placement_stocks: &'a [(ItemStack, i32)],
}

impl<'a, 'w> Resolver<'a, 'w> {
    pub(super) fn new(
        game: &'a Game<'w>,
        stocks: &'a Stacked,
        placement_stocks: &'a [(ItemStack, i32)],
    ) -> Self {
        Self {
            game,
            stocks,
            placement_stocks,
        }
    }

    pub(super) fn resolve(&self, slot: &Slot) -> Vec<ItemStack> {
        let items = self.game.items();
        match slot {
            Slot::Empty => Vec::new(),
            Slot::Item(id) => items
                .get(id)
                .map(|_| ItemStack::new(id, 1))
                .into_iter()
                .collect(),
            Slot::ItemStack(stack) => stack.try_into_engine(items).ok().into_iter().collect(),
            Slot::AnyFuel => items
                .registry_order
                .iter()
                .filter(|id| items.get(id).is_some_and(|i| i.fuel.is_some()))
                .map(|id| ItemStack::new(id, 1))
                .collect(),
            Slot::Tag(tag) => items
                .item_tag_order
                .get(tag)
                .into_iter()
                .flatten()
                .map(|id| ItemStack::new(id, 1))
                .collect(),
            Slot::Composite(children) => children
                .iter()
                .flat_map(|child| self.resolve(child))
                .collect(),
            Slot::WithRemainder { input, .. } => self.resolve(input),
            Slot::WithAnyPotion(source) => self
                .resolve(source)
                .into_iter()
                .flat_map(|stack| {
                    horizon_client::brewing::POTIONS
                        .iter()
                        .map(move |p| stack.clone().with_potion(&format!("minecraft:{}", p.id)))
                })
                .collect(),
            Slot::OnlyWithComponent { source, component } => self
                .resolve(source)
                .into_iter()
                .filter(|stack| stack.carries_component(items.get(&stack.item), component))
                .collect(),
            Slot::Dyed { dye, target } => {
                let targets = self.resolve(target);
                self.resolve(dye)
                    .iter()
                    .map(|stack| {
                        let name = stack
                            .data()
                            .and_then(|data| data.other("minecraft:dye"))
                            .and_then(|tag| match tag {
                                horizon_core::nbt::Tag::String(value) => {
                                    Some(value.to_string_lossy())
                                }
                                _ => None,
                            })
                            .or_else(|| items.get(&stack.item).and_then(|info| info.dye.clone()))
                            .unwrap_or_else(|| "white".into());
                        horizon_client::block_entity_render::DYES
                            .iter()
                            .position(|(id, _)| *id == name)
                            .unwrap_or(0)
                    })
                    .flat_map(|dye| targets.iter().map(move |stack| dye_stack(stack, dye)))
                    // SlotDisplay.applyDemoTransformation cycles targets fastest,
                    // then keeps at most 16 successful transformations.
                    .take(16)
                    .collect()
            }
            Slot::SmithingTrim {
                base,
                material,
                pattern,
            } => {
                let materials = self.resolve(material);
                let pattern =
                    horizon_client::crafting::RecipeTrimPattern::Reference(pattern.clone());
                let bases = self.resolve(base);
                if bases.is_empty() || materials.is_empty() {
                    return Vec::new();
                }
                use horizon_core::random::RandomSource;
                // Vanilla seeds this demo from display identity, samples at most
                // 256 pairs, and retains the first 16 successful trims.
                let mut random =
                    horizon_core::random::LegacyRandom::new(slot as *const Slot as usize as i64);
                (0..256)
                    .filter_map(|_| {
                        let base = random
                            .next_int_bound(i32::try_from(bases.len()).ok()?)
                            .ok()? as usize;
                        let material = random
                            .next_int_bound(i32::try_from(materials.len()).ok()?)
                            .ok()? as usize;
                        horizon_client::stations::apply_trim(
                            &self.game.stations().enchantments,
                            &bases[base],
                            &materials[material],
                            &pattern,
                        )
                    })
                    .take(16)
                    .collect()
            }
        }
    }

    pub(super) fn craftable(&self, ingredients: &[Ingredient]) -> bool {
        self.stocks
            .can_craft(&ingredients.iter().collect::<Vec<_>>())
    }

    pub(super) fn search_text(&self, stacks: &[ItemStack]) -> String {
        stacks
            .iter()
            .map(|stack| horizon_client::stations::hover_name(&self.game.pack.names, stack))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
}

fn dye_stack(target: &ItemStack, dye: usize) -> ItemStack {
    let mut stack = target.clone();
    stack.count = 1;
    let current = stack.data().and_then(|data| data.dyed_color);
    stack.data_mut().dyed_color = Some(horizon_client::crafting::apply_dyes(current, &[dye]));
    stack
}

impl horizon_client::recipe_book::mod_presentation::DisplayResolver for Resolver<'_, '_> {
    fn resolve(&self, slot: &Slot) -> Vec<ItemStack> {
        Resolver::resolve(self, slot)
    }
    fn craftable(&self, recipe: &str, _: &Display, ingredients: &[Ingredient]) -> bool {
        if self
            .game
            .native_screen_menu()
            .is_some_and(|menu| mod_api::RecipeBookLayout::declaring_mod(menu).is_some())
        {
            let Some(plan) = self
                .game
                .native_net
                .placement
                .view
                .as_ref()
                .and_then(|view| view.index.placement(recipe).ok())
            else {
                return false;
            };
            let ingredients = &plan.requirements().ingredients;
            let mut counts: Vec<_> = self
                .placement_stocks
                .iter()
                .map(|(_, amount)| *amount)
                .collect();
            return horizon_client::menu::place::can_craft_matching(
                &mut counts,
                ingredients.len(),
                |candidate, ingredient| {
                    if ingredients[ingredient]
                        .test_in(&self.game.loop_context, &self.placement_stocks[candidate].0)
                    {
                        i32::MAX
                    } else {
                        0
                    }
                },
            );
        }
        Resolver::craftable(self, ingredients)
    }
    fn search_text(&self, results: &[ItemStack]) -> String {
        Resolver::search_text(self, results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn can_craft(stocks: &[ItemStack], ingredients: &[Ingredient]) -> bool {
        let mut stacked = Stacked::default();
        for stack in stocks {
            stacked.account_stack(stack, stack.count);
        }
        stacked.can_craft(&ingredients.iter().collect::<Vec<_>>())
    }

    #[test]
    fn dye_blends_existing_color_and_preserves_other_components() {
        let mut target = ItemStack::new("test:target", 7).with_potion("minecraft:water");
        target.data_mut().dyed_color = Some(0xB02E26);
        let dyed = dye_stack(&target, 11);
        assert_eq!(dyed.count, 1);
        assert_eq!(dyed.data().unwrap().dyed_color, Some(0xAD5398));
        let mut expected = target.clone();
        expected.count = 1;
        expected.data_mut().dyed_color = dyed.data().unwrap().dyed_color;
        assert_eq!(dyed, expected);
        assert_eq!(target.count, 7);
    }
    #[test]
    fn duplicate_inputs_consume_capacity_and_reassign_alternatives() {
        let ingredients = [
            Ingredient::of(vec!["test:a".into(), "test:b".into()]),
            Ingredient::of(vec!["test:a".into()]),
        ];
        assert!(!can_craft(&[ItemStack::new("test:a", 1)], &ingredients));
        assert!(can_craft(&[ItemStack::new("test:a", 2)], &ingredients));
        assert!(can_craft(
            &[ItemStack::new("test:a", 1), ItemStack::new("test:b", 1)],
            &ingredients
        ));
    }
}
