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
    /// `LivingEntity.canGlide`: some worn piece `gliding_pieces` lets glide,
    /// or a stack in a hand that glides there (`canGlideUsing` walks every
    /// `EquipmentSlot`).
    pub(super) fn glider_worn(&self) -> bool {
        let items = self.res.items();
        let hand = |stack: Option<&ItemStack>, slot: &str| {
            stack.is_some_and(|s| horizon_client::item_sync::can_glide_using(s, slot, items))
        };
        gliding_pieces(items, &self.armor).contains(&true)
            || hand(self.inventory[self.selected].as_ref(), "mainhand")
            || hand(self.offhand.as_ref(), "offhand")
    }

    /// `LivingEntity.updateFallFlying`: the glide's wear. `Util.getRandom`
    /// picks one of the gliders that will not break (from the entity's own
    /// random), and
    /// `ItemStack.hurtAndBreak` takes `points` of its durability (Unbreaking
    /// may save it). Callers skip creative.
    pub(super) fn wear_glider(&mut self, points: u32) {
        let items = self.res.items();
        let gliding = gliding_pieces(items, &self.armor);
        let hand = |stack: Option<&ItemStack>, slot: &str| {
            stack.is_some_and(|s| horizon_client::item_sync::can_glide_using(s, slot, items))
        };
        // `EquipmentSlot.VALUES` order: mainhand, offhand, feet, legs, chest,
        // head (`armor` is head, chest, legs, feet). `Ok` is a hand slot.
        let mut slots: Vec<std::result::Result<usize, usize>> = Vec::new();
        if hand(self.inventory[self.selected].as_ref(), "mainhand") {
            slots.push(Ok(self.selected));
        }
        if hand(self.offhand.as_ref(), "offhand") {
            slots.push(Ok(shield::OFFHAND_SLOT));
        }
        slots.extend([3, 2, 1, 0].into_iter().filter(|&i| gliding[i]).map(Err));
        if slots.is_empty() {
            return;
        }
        // `Util.getRandom(slotsWithGliders, this.random)`: the entity's own
        // random, which never touches the level's stream.
        let pick = random_source::entity().next_int_bound(slots.len() as i32);
        let pick = slots[pick.unwrap_or(0) as usize];
        let stack = match pick {
            Ok(slot) => self.stack_in(slot).cloned(),
            Err(i) => self.armor[i].clone(),
        };
        if let Some(stack) = stack {
            let loss = self.wear(&stack, points);
            let worn = match pick {
                Ok(slot) => self.stack_in_mut(slot).as_mut(),
                Err(i) => self.armor[i].as_mut(),
            };
            if let Some(piece) = worn {
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

    /// `PowderSnowBlock.canEntityWalkOnPowderSnow`'s boots half: leather boots
    /// on the feet.
    pub(crate) fn wears_leather_boots(&self) -> bool {
        self.armor[3]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:leather_boots")
    }

    pub(super) fn held_slot(&self) -> usize {
        match self.hand {
            Hand::Main => self.selected,
            Hand::Off => shield::OFFHAND_SLOT,
        }
    }

    /// `LivingEntity.getUsedItemHand`: the hand of the item in use.
    pub(super) fn used_hand(&self) -> Option<Hand> {
        self.using.map(|(slot, _)| {
            if slot == shield::OFFHAND_SLOT {
                Hand::Off
            } else {
                Hand::Main
            }
        })
    }

    /// Run `f` with `hand` acting (`held`, `consume_held`, `swing_arm`).
    pub(super) fn with_hand<T>(&mut self, hand: Hand, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = std::mem::replace(&mut self.hand, hand);
        let out = f(self);
        self.hand = prior;
        out
    }

    /// Inventory.add: returns the stack that did not fit.
    pub(super) fn give(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
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
