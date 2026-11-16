use super::*;
use crate::game::durability::Wearer;

#[cfg(test)]
thread_local! {
    static DISPATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Game<'_> {
    #[cfg(test)]
    pub(in crate::game) fn damage_item_dispatches(&self) -> usize {
        DISPATCHES.with(std::cell::Cell::get)
    }

    /// `IItemExtension.damageItem`: `stack`, worn or used by `entity`, is
    /// about to lose `amount` points of durability (`ItemStack.hurtAndBreak`),
    /// after Unbreaking. Change `amount` (0 or less leaves the item whole),
    /// set `canceled` for no wear, or set `break_now` to break the item.
    /// Server-side.
    pub(in crate::game) fn native_damage_item(
        &mut self,
        stack: &ItemStack,
        entity: Option<LivingRef>,
        amount: &mut i32,
        canceled: &mut bool,
        break_now: &mut bool,
    ) {
        #[cfg(test)]
        DISPATCHES.with(|count| count.set(count.get() + 1));
        self.fire_damage_item(stack, entity, amount, canceled, break_now);
    }
}

impl Game<'_> {
    /// The parts' answer for `loss` points of wear on `stack`, worn or used by
    /// `wearer`: the points it takes, or all the durability it has left when a
    /// part breaks it now (the vanilla break then runs where the wear lands).
    pub(in crate::game) fn damage_item_loss(
        &mut self,
        stack: &ItemStack,
        loss: u32,
        wearer: Wearer,
    ) -> u32 {
        let entity = match wearer {
            Wearer::Player => self.effect_living_ref(None),
            Wearer::Living(id) => self.effect_living_ref(Some(id as i32)),
            Wearer::Nobody => None,
        };
        let mut points = loss.min(i32::MAX as u32) as i32;
        let (mut canceled, mut break_now) = (false, false);
        self.native_damage_item(stack, entity, &mut points, &mut canceled, &mut break_now);
        if break_now {
            let max = stack.max_damage(self.items().get(&stack.item));
            let damage = stack.damage.min(i32::MAX as u32) as i32;
            return (max.saturating_sub(damage)).max(1) as u32;
        }
        if canceled { 0 } else { points.max(0) as u32 }
    }
}
