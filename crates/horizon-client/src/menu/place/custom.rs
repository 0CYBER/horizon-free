//! Detached ServerPlaceRecipe planning for checked, arbitrary mod-menu roles.
use super::{Env, Kind, Menu, PLAYER, PostPlace, tidy, usable_for_crafting};
use super::{
    custom_plan::{MenuPlacement, PlacementRefusal as Refused},
    picker::Picker,
};
use crate::items::ItemStack;

#[cfg(test)]
#[path = "custom_tests.rs"]
mod tests;

impl Menu {
    /// Shared vanilla calculateAmountToCraft, including an already matching grid.
    pub(super) fn recipe_place_amount(
        &self,
        inputs: &[usize],
        use_max: bool,
        matches: bool,
        biggest: i32,
    ) -> Option<i32> {
        Some(if use_max {
            biggest
        } else if matches {
            match inputs
                .iter()
                .filter_map(|slot| self.slot(*slot).map(|stack| stack.count))
                .min()
            {
                Some(count) => i32::try_from(count).ok()?.checked_add(1)?,
                None => i32::MAX,
            }
        } else {
            1
        })
    }
    /// Apply only a complete plan to this detached menu. External storage is
    /// still the server transaction's responsibility; no world writes occur here.
    pub fn place_custom_recipe(
        &mut self,
        env: &Env,
        plan: &MenuPlacement,
        use_max: bool,
        allow_drop: bool,
        guard: impl Fn() -> bool,
    ) -> Result<PostPlace, Refused> {
        self.touch();
        if !guard() {
            return Err(Refused::Changed);
        }
        if !matches!(self.kind(), Kind::Mod(menu) if menu.id == plan.menu) {
            return Err(Refused::Unavailable);
        }
        for &slot in &plan.clear {
            if self.slot(slot).is_some() {
                let allowed = self.may_pickup(env, slot);
                if !guard() {
                    return Err(Refused::Changed);
                }
                if !allowed {
                    return Err(Refused::Denied);
                }
            }
        }
        let mut prototypes: Vec<ItemStack> = Vec::new();
        let mut amounts: Vec<i32> = Vec::new();
        {
            let mut account = |stack: &ItemStack, simple: bool| -> Result<(), Refused> {
                if simple && !usable_for_crafting(stack) {
                    return Ok(());
                }
                let count = i32::try_from(stack.count.min(Self::max_stack(env, stack)))
                    .map_err(|_| Refused::InvalidLayout)?;
                if count == 0 {
                    return Ok(());
                }
                if let Some(index) = prototypes
                    .iter()
                    .position(|p| p.same_item_same_components(stack))
                {
                    amounts[index] = amounts[index]
                        .checked_add(count)
                        .ok_or(Refused::InvalidLayout)?;
                } else {
                    prototypes.push(ItemStack {
                        count: 1,
                        ..stack.clone()
                    });
                    amounts.push(count);
                }
                Ok(())
            };
            for stack in self.state.containers[PLAYER][..36].iter().flatten() {
                account(stack, true)?;
            }
            for stack in self.state.sources.iter().flatten().flatten() {
                account(stack, true)?;
            }
            for &slot in &plan.inputs {
                if let Some(stack) = self.slot(slot) {
                    account(stack, false)?;
                }
            }
        }
        let ingredients = &plan.placement.ingredients;
        let mut target = vec![0; ingredients.len()];
        for &(ingredient, slot) in &plan.cells {
            target[ingredient] = slot;
        }
        let matches =
            plan.inputs
                .iter()
                .try_fold(true, |matches, slot| -> Result<bool, Refused> {
                    if !matches {
                        return Ok(false);
                    }
                    Ok(match plan.cells.iter().find(|(_, at)| at == slot) {
                        Some((ingredient, _)) => match self.slot(*slot) {
                            Some(stack) => ingredients[*ingredient]
                                .test_in_guarded(env.loop_context, stack, &guard)
                                .ok_or(Refused::Changed)?,
                            None => false,
                        },
                        None => self.slot(*slot).is_none(),
                    })
                })?;
        let mut next = self.clone();
        let dropped = next.state.dropped.len();
        for &slot in &plan.clear {
            if let Some(mut stack) = next.slot(slot).cloned() {
                next.load_slot(slot, None);
                next.place_back(env, &mut stack);
            }
        }
        if !guard() {
            return Err(Refused::Changed);
        }
        if !allow_drop && next.state.dropped.len() != dropped {
            return Err(Refused::NoRoom);
        }
        if let Some(result) = plan.virtual_result {
            next.load_slot(result, None);
        }
        let mut compatible = Vec::with_capacity(prototypes.len() * ingredients.len());
        let mut limits = Vec::with_capacity(prototypes.len() * ingredients.len());
        for prototype in &prototypes {
            for (ingredient, slot) in ingredients.iter().zip(&target) {
                let accepted = ingredient
                    .test_in_guarded(env.loop_context, prototype, &guard)
                    .ok_or(Refused::Changed)?;
                compatible.push(if accepted { i32::MAX } else { 0 });
                let allowed = if accepted {
                    let allowed = next.may_place(env, *slot, prototype);
                    if !guard() {
                        return Err(Refused::Changed);
                    }
                    allowed
                } else {
                    false
                };
                let cap = if allowed {
                    next.slot_max(env, *slot, prototype)
                        .min(Self::max_stack(env, prototype))
                } else {
                    0
                };
                limits.push(i32::try_from(cap).map_err(|_| Refused::InvalidLayout)?);
            }
        }
        let raw_available =
            super::can_craft_matching(&mut amounts, ingredients.len(), |candidate, ingredient| {
                compatible[candidate * ingredients.len() + ingredient]
            });
        if !raw_available {
            if plan.virtual_result.is_some() {
                next.recompute(env);
            }
            if !guard() {
                return Err(Refused::Changed);
            }
            self.install_state(next);
            return Ok(PostPlace::Ghost);
        }
        let mut picker = Picker::new(&mut amounts, ingredients.len(), |candidate, ingredient| {
            limits[candidate * ingredients.len() + ingredient]
        });
        let biggest = picker.try_pick_all(i32::MAX, None);
        if biggest == 0 {
            return Err(Refused::Denied);
        }
        let amount = self
            .recipe_place_amount(&plan.inputs, use_max, matches, biggest)
            .ok_or(Refused::InvalidLayout)?;
        if amount > biggest {
            return Ok(PostPlace::Nothing);
        }
        let mut selected = Vec::new();
        if !picker.try_pick(amount, Some(&mut selected)) {
            return Err(Refused::Changed);
        }
        for (ingredient, candidate) in selected.into_iter().enumerate() {
            let prototype = &prototypes[candidate];
            let mut remaining = amount as u32;
            for stack in next.state.containers[PLAYER][..36]
                .iter_mut()
                .chain(next.state.sources.iter_mut().flatten())
            {
                if let Some(have) = stack
                    // Only eligible stock contributed to this exact prototype's
                    // budget. Cleared grid inputs remain usable even when named,
                    // enchanted or damaged; component equality preserves that budget.
                    && have.same_item_same_components(prototype)
                {
                    let taken = remaining.min(have.count);
                    have.count -= taken;
                    remaining -= taken;
                    tidy(stack);
                }
                if remaining == 0 {
                    break;
                }
            }
            if remaining != 0 {
                return Err(Refused::Changed);
            }
            next.load_slot(
                target[ingredient],
                Some(ItemStack {
                    count: amount as u32,
                    ..prototype.clone()
                }),
            );
        }
        if plan.virtual_result.is_some() {
            next.recompute(env);
        }
        if !guard() {
            return Err(Refused::Changed);
        }
        self.install_state(next);
        Ok(PostPlace::Nothing)
    }
}
