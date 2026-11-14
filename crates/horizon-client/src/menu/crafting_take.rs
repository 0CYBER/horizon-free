//! Result-slot take handling, including pre-take revalidation and remainders.
use super::*;
impl Menu {
    pub(super) fn crafting_result_is_current(&self, env: &Env, slot: usize) -> bool {
        if !env.recipes.has_mod_crafting() {
            return true;
        }
        env.recipes
            .find_with(
                env.loop_context,
                &self.state.containers[GRID],
                self.state.grid_size,
                &|id| {
                    env.maps
                        .and_then(|maps| maps.state(id))
                        .map(|(_, scale)| scale)
                },
            )
            .is_some_and(|(_, result)| {
                self.slot(slot).is_some_and(|shown| {
                    shown.count == result.count && shown.same_item_same_components(&result)
                })
            })
    }

    /// `ResultSlot.onTake` and the other `Slot.onTake` overrides.
    pub(super) fn on_take(&mut self, env: &Env, s: usize, taken: &mut ItemStack) {
        self.touch();
        if self.state.slots[s].role == Role::Output {
            self.furnace_take(env, taken);
            return;
        }
        if self.state.slots[s].role == Role::Potion {
            if self.state.brewed_potion_hook && env.loop_context.side() == LoopSide::Server {
                self.state.brewed_potion_takes.push(taken.clone());
            }
            // BrewingStandMenu.PotionSlot.onTake: a stack with potion contents.
            if matches!(
                &*taken.item,
                "minecraft:potion" | "minecraft:splash_potion" | "minecraft:lingering_potion"
            ) {
                self.state.brewed.push(taken.clone());
            }
            return;
        }
        if self.state.slots[s].role == Role::MerchantResult {
            self.merchant_check_take_achievements(taken);
            self.merchant_take(env);
            return;
        }
        if self.state.slots[s].role == Role::StoneResult {
            // StonecutterMenu's result onTake: onCraftedBy, awardUsedRecipes,
            // one input used up, the result again for the selected recipe.
            self.crafted_by(taken, taken.count as i32);
            if let Some(r) = self.state.stone.used {
                self.state.crafted.push((
                    env.station_recipes.stonecutting[r].id.clone(),
                    vec![self.state.containers[MAIN][0].clone()],
                ));
            }
            if self.remove(env, 0, 1).is_some() {
                self.stone_setup_result(env, self.state.data[0]);
            }
            self.take_sound("minecraft:ui.stonecutter.take_result");
            return;
        }
        if self.state.slots[s].role == Role::GrindResult {
            if self.state.grind_hook {
                self.state.grind_take = Some(GrindstoneTake {
                    top: self.state.containers[MAIN][0].clone(),
                    bottom: self.state.containers[MAIN][1].clone(),
                    xp: self.state.grind_xp,
                    new_top: None,
                    new_bottom: None,
                });
                return;
            }
            // GrindstoneMenu's result onTake: the experience (from the inputs
            // before they go), then both inputs cleared.
            self.state.ground = Some(crate::stations::grindstone_xp_base(
                env.enchantments,
                self.state.containers[MAIN][0].as_ref(),
                self.state.containers[MAIN][1].as_ref(),
            ));
            self.put(env, 0, None);
            self.put(env, 1, None);
            return;
        }
        if self.state.slots[s].role == Role::CombineResult && self.state.kind == Kind::Anvil {
            if self.state.anvil_hook {
                self.state.anvil_craft = Some(AnvilCraft {
                    left: self.state.containers[MAIN][0].clone(),
                    right: self.state.containers[MAIN][1].clone(),
                    output: taken.clone(),
                });
                return;
            }
            self.anvil_take(env);
            return;
        }
        if self.state.slots[s].role == Role::CombineResult {
            // SmithingMenu.onTake: onCraftedBy, the `ItemSmithedEvent` hook,
            // then awardUsedRecipes, then one of each input.
            if self.state.kind == Kind::Smithing {
                self.crafted_by(taken, taken.count as i32);
                let inputs = [0, 1, 2].map(|i| self.state.containers[MAIN][i].clone());
                if self.state.item_smithed_hook
                    && env.loop_context.side() == LoopSide::Server
                    && taken.count > 0
                {
                    self.state
                        .item_smithed
                        .push((inputs.clone(), taken.clone()));
                }
                if let Some((id, _)) = env.station_recipes.smithing_recipe(
                    env.loop_context,
                    env.enchantments,
                    inputs[0].as_ref(),
                    inputs[1].as_ref(),
                    inputs[2].as_ref(),
                ) {
                    self.state.crafted.push((id.to_owned(), inputs.to_vec()));
                }
            }
            for i in 0..3 {
                if let Some(mut stack) = self.state.containers[MAIN][i].clone() {
                    stack.count -= 1;
                    self.put(env, i, Some(stack));
                }
            }
            self.state.combined = true;
            return;
        }
        if self.state.slots[s].role == Role::CartoResult {
            // CartographyTableMenu's result onTake: one of each input, the
            // take sound, then Slot.onTake's setChanged. The taken stack's
            // onCraftedBy is the caller's (map_crafted).
            self.remove(env, 0, 1);
            self.remove(env, 1, 1);
            self.take_sound("minecraft:ui.cartography_table.take_result");
            self.slot_changed(env, s);
            return;
        }
        if self.state.slots[s].role == Role::LoomResult {
            // LoomMenu's result onTake: one banner and one dye are used up.
            self.remove(env, 0, 1);
            self.remove(env, 1, 1);
            if self.state.containers[MAIN][0].is_none() || self.state.containers[MAIN][1].is_none()
            {
                self.state.data[0] = -1;
            }
            self.take_sound("minecraft:ui.loom.take_result");
            return;
        }
        if matches!(self.state.kind, Kind::CrafterBlock) && self.state.slots[s].container == MAIN {
            // Slot.onTake -> CrafterSlot.setChanged -> the result refresh.
            self.slot_changed(env, s);
            return;
        }
        if self.state.slots[s].role != Role::CraftResult {
            return;
        }
        // ResultSlot.checkTakeAchievements -> Item.onCraftedBy mutates the
        // actual taken stack, before the inputs are consumed.
        if taken.count > 0 {
            Self::map_crafted(env.maps, taken);
        }
        let n = self.state.grid_size;
        let grid = self.state.containers[GRID].clone();
        let map_scale = |id| {
            env.maps
                .and_then(|maps| maps.state(id))
                .map(|(_, scale)| scale)
        };
        if let Some((id, _)) = env
            .recipes
            .find_with(env.loop_context, &grid, n, &map_scale)
        {
            let id = crate::crafting::qualified(id);
            let made = (id.clone(), taken.clone(), grid.clone());
            self.state.crafted.push((id, grid.clone()));
            self.state.item_crafted.push(made);
        }
        let remaining = match env
            .recipes
            .find_with(env.loop_context, &grid, n, &map_scale)
        {
            Some((id, _))
                if env.loop_context.side()
                    == horizon_world::native_load_context::LoopSide::Server
                    || !matches!(
                        env.recipes.recipe(id),
                        Some(crate::crafting::Recipe::Mod(_))
                    ) =>
            {
                env.recipes.remainders(id, &grid, n, &|s| env.remainder(s))
            }
            Some(_) => grid
                .iter()
                .map(|cell| cell.as_ref().and_then(|s| env.remainder(s)))
                .collect(),
            None => return,
        };
        for cell in 0..n * n {
            let mut here = self.state.containers[GRID][cell].take();
            if let Some(stack) = here.as_mut() {
                stack.count -= 1;
            }
            tidy(&mut here);
            self.state.containers[GRID][cell] = here;
            if let Some(replacement) = remaining.get(cell).cloned().flatten() {
                match self.state.containers[GRID][cell].as_mut() {
                    None => self.state.containers[GRID][cell] = Some(replacement),
                    Some(have) if have.same_item_same_components(&replacement) => {
                        let mut merged = replacement;
                        merged.count += have.count;
                        self.state.containers[GRID][cell] = Some(merged);
                    }
                    Some(_) => {
                        let mut left = replacement;
                        if !self.inventory_add(env, &mut left) {
                            self.drop_stack(&left);
                        }
                    }
                }
            }
        }
        self.recompute(env);
    }
}
