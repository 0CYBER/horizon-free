//! The player's hands and what it wears: the stack in the acting hand (`getItemInHand`), the hand of
//! the item in use and running code with the other hand acting, `Inventory.add`, and the worn glider
//! and freeze-immune gear.
use super::*;

impl Game<'_> {
    /// The stack in the hand that acts (`getItemInHand(hand)`).
    pub(super) fn held(&self) -> Option<&ItemStack> {
        self.stack_in(self.held_slot())
    }

    pub(super) fn held_mut(&mut self) -> &mut Option<ItemStack> {
        self.stack_in_mut(self.held_slot())
    }

    /// The slot of the hand that acts: the selected one, or `OFFHAND_SLOT`.
    /// `LivingEntity.canGlide`: some worn piece, or a stack in a hand, glides
    /// there (`canGlideUsing` walks every `EquipmentSlot`: `gliding_slots`).
    pub(super) fn glider_worn(&self) -> bool {
        super::glide_wear::gliding_slots(
            self.res.items(),
            &self.armor,
            self.inventory[self.selected].as_ref(),
            self.offhand.as_ref(),
        )
        .next()
        .is_some()
    }

    /// `LivingEntity.updateFallFlying`: the glide's wear. `Util.getRandom`
    /// picks one of the gliders that will not break (from the entity's own
    /// random), and
    /// `ItemStack.hurtAndBreak` takes `points` of its durability (Unbreaking
    /// may save it). Callers skip creative.
    pub(super) fn wear_glider(&mut self, points: u32) {
        // Every 20th glide tick only, so collecting the candidates here is fine.
        let mut pieces: Vec<usize> = super::glide_wear::gliding_slots(
            self.res.items(),
            &self.armor,
            self.inventory[self.selected].as_ref(),
            self.offhand.as_ref(),
        )
        .collect();
        if pieces.is_empty() {
            return;
        }
        // `Util.getRandom(slotsWithGliders, this.random)` over `EquipmentSlot.VALUES` order:
        // mainhand, offhand, feet, legs, chest, head; the entity's own random, which never
        // touches the level's stream.
        pieces.sort_unstable_by_key(|&slot| [5, 4, 3, 2, 0, 1][slot]);
        let pick = random_source::entity().next_int_bound(pieces.len() as i32);
        let slot = pieces[pick.unwrap_or(0) as usize];
        let stack = match slot {
            4 => self.inventory[self.selected].clone(),
            5 => self.offhand.clone(),
            i => self.armor[i].clone(),
        };
        // `ItemStack.hurtAndBreak` skips a stack that isn't `isDamageableItem` (no `damage`
        // component, say), though it may still glide.
        if let Some(stack) = stack
            && horizon_client::stations::is_damageable(self.res.items(), &stack)
        {
            let loss = self.wear(&stack, points);
            let target = match slot {
                4 => &mut self.inventory[self.selected],
                5 => &mut self.offhand,
                i => &mut self.armor[i],
            };
            if let Some(piece) = target.as_mut() {
                piece.damage += loss;
            }
        }
    }

    /// `LivingEntity.canFreeze`'s armor half: any worn piece in
    /// `freeze_immune_wearables`.
    pub(crate) fn wears_freeze_immune(&self) -> bool {
        let immune = self
            .res
            .items()
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        self.armor
            .iter()
            .flatten()
            .any(|s| immune.is_some_and(|t| t.contains(&*s.item)))
    }

    pub(super) fn held_slot(&self) -> usize {
        match self.hand {
            Hand::Main => self.selected,
            Hand::Off => shield::OFFHAND_SLOT,
        }
    }

    /// `LivingEntity.getUsedItemHand`: the hand of the item in use.
    pub(super) fn used_hand(&self) -> Option<Hand> {
        item_use::hand_of(self.using)
    }

    /// Run `f` with `hand` acting (`held`, `consume_held`, `swing_arm`).
    pub(super) fn with_hand<T>(&mut self, hand: Hand, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = std::mem::replace(&mut self.hand, hand);
        let out = f(self);
        self.hand = prior;
        out
    }

    /// Inventory.add: returns the stack that did not fit.
    pub(super) fn add_stack_to_inventory(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
        let tutorial_item = matches!(
            self.tutorial.step,
            tutorial::Step::FindTree | tutorial::Step::PunchTree | tutorial::Step::CraftPlanks
        )
        .then(|| stack.clone());
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack);
        let before = stack.count;
        let left = inventory_add(
            &mut self.inventory,
            &mut self.offhand,
            self.selected,
            max,
            &mut stack,
        );
        if stack.count < before {
            self.tutorial_get_item(tutorial_item.as_ref());
        }
        left.then_some(stack)
    }
}

/// The item of the stack in `selected` (`Player.tick`'s main-hand item), borrowed: a guest's
/// tick compares it with the last one and clones it only when it changed.
pub(super) fn held_attack_item(
    inventory: &[Option<ItemStack>],
    selected: usize,
) -> Option<&Arc<str>> {
    inventory.get(selected)?.as_ref().map(|s| &s.item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str) -> ItemStack {
        ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        }
    }

    #[test]
    fn held_attack_item_matches_the_cloned_stack() {
        let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
        inventory[0] = Some(stack("minecraft:stone"));
        inventory[3] = Some(stack("minecraft:diamond_sword"));
        for selected in [0, 1, 3, 35, 36, 1000] {
            // The clone the tick used to make, as the reference.
            let cloned = inventory.get(selected).cloned().flatten().map(|s| s.item);
            assert_eq!(held_attack_item(&inventory, selected).cloned(), cloned);
        }
        assert_eq!(held_attack_item(&[], 0), None);
    }
}
