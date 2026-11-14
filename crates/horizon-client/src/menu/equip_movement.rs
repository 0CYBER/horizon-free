//! Equipment-aware movement owns a source slot while traversing destinations.
use super::*;
use crate::mobs::equip::EquipDecision;
use std::ops::Deref;
trait MoveSource: Deref<Target = ItemStack> {
    fn count_mut(&mut self) -> &mut u32;
    fn take_portion(&mut self, count: u32) -> ItemStack;
}
struct Plain<'a>(&'a mut ItemStack);
impl Deref for Plain<'_> {
    type Target = ItemStack;
    fn deref(&self) -> &ItemStack {
        self.0
    }
}
impl MoveSource for Plain<'_> {
    fn count_mut(&mut self) -> &mut u32 {
        &mut self.0.count
    }
    fn take_portion(&mut self, count: u32) -> ItemStack {
        split(self.0, count)
    }
}
impl MoveSource for EquipDecision<'_> {
    fn count_mut(&mut self) -> &mut u32 {
        EquipDecision::count_mut(self).expect("live equipment input")
    }
    fn take_portion(&mut self, count: u32) -> ItemStack {
        self.split(count)
    }
}
impl Menu {
    pub(super) fn move_to_equip_source(
        &mut self,
        env: &Env,
        stack: &mut ItemStack,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> bool {
        if env.player_entity_id.is_none()
            || !env.can_equip.is_some_and(|parts| parts.listens(stack))
            || !(start..end).any(|slot| matches!(self.state.slots[slot].role, Role::Armor(_)))
        {
            return self.move_from_source(env, &mut Plain(stack), start, end, backwards);
        }
        let location = stack as *const _ as usize as u64;
        let mut slot = Some(stack.clone());
        let mut source = EquipDecision::new(location, &mut slot);
        if !env.can_equip.expect("observed equip input").prepare(
            &source,
            env.player_entity_id.unwrap() as u32,
            "minecraft:player",
        ) {
            return false;
        }
        let moved = self.move_from_source(env, &mut source, start, end, backwards);
        *stack = source
            .remove()
            .expect("equipment movement retains its input");
        moved
    }
    fn move_from_source<S: MoveSource>(
        &mut self,
        env: &Env,
        stack: &mut S,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> bool {
        self.touch();
        let mut changed = false;
        let order = |_: ()| -> Vec<usize> {
            if backwards {
                (start..end).rev().collect()
            } else {
                (start..end).collect()
            }
        };
        if Self::stackable(env, stack) {
            for dest in order(()) {
                if stack.count == 0 {
                    break;
                }
                // A mod menu's result (unless a mod's hook opens it) and a
                // hidden slot get no top-up either, as a furnace's quick move
                // never aims at its result.
                let refuses = if self.state.slots[dest].role == Role::Hidden {
                    true
                } else if let Some(id) = self.registry_id() {
                    !apply_menu_slot_rules(
                        env.slot_rules,
                        id,
                        self.state.slots[dest].index,
                        stack,
                        self.state.slots[dest].role != Role::ModResult,
                        CONTAINER_MAX,
                        Self::max_stack(env, stack),
                    )
                    .0
                } else {
                    false
                };
                let max = match self.slot(dest) {
                    Some(target) if !refuses && target.same_item_same_components(stack) => {
                        self.slot_max(env, dest, target)
                    }
                    _ => continue,
                };
                let slot = self.state.slots[dest];
                let target = self.state.containers[slot.container][slot.index]
                    .as_mut()
                    .expect("checked");
                let total = target.count + stack.count;
                if total <= max {
                    *stack.count_mut() = 0;
                    target.count = total;
                    changed = true;
                    self.slot_changed(env, dest);
                } else if target.count < max {
                    *stack.count_mut() -= max - target.count;
                    target.count = max;
                    changed = true;
                    self.slot_changed(env, dest);
                }
            }
        }
        if stack.count > 0 {
            for dest in order(()) {
                if self.has_item(dest) {
                    continue;
                }
                let proof = self.equip_placement(env, dest, stack);
                if proof.allowed {
                    let max = proof.max;
                    if !proof.can_apply(self, env, stack) {
                        return changed;
                    }
                    let part = stack.take_portion(stack.count.min(max));
                    self.set_by_player(env, dest, Some(part));
                    changed = true;
                    break;
                }
            }
        }
        changed
    }
}

impl Menu {
    pub(super) fn insert_equip_source(
        &mut self,
        env: &Env,
        s: usize,
        input: &mut Option<ItemStack>,
        amount: u32,
    ) {
        self.touch();
        let Some(stack) = input.as_ref() else { return };
        if let (Some(parts), Some(player)) = (env.can_equip, env.player_entity_id)
            && parts.listens(stack)
            && matches!(self.state.slots[s].role, Role::Armor(_))
        {
            let location = stack as *const _ as usize as u64;
            let mut source = EquipDecision::new(location, input);
            if !parts.prepare(&source, player as u32, "minecraft:player") {
                return;
            }
            self.insert_from_source(env, s, &mut source, amount);
        } else {
            self.insert_from_source(env, s, &mut Plain(input.as_mut().unwrap()), amount);
        }
        tidy(input);
    }
    fn insert_from_source<S: MoveSource>(
        &mut self,
        env: &Env,
        s: usize,
        stack: &mut S,
        amount: u32,
    ) {
        let proof = self.equip_placement(env, s, stack);
        self.insert_from_source_with_placement(env, s, stack, amount, proof);
    }
    fn insert_from_source_with_placement<S: MoveSource>(
        &mut self,
        env: &Env,
        s: usize,
        stack: &mut S,
        amount: u32,
        proof: super::equippable::EquipPlacement,
    ) {
        if !proof.allowed {
            return;
        }
        let have = self.slot(s).map_or(0, |c| c.count);
        let max = proof.max;
        let transferable = amount.min(stack.count).min(max.saturating_sub(have));
        if transferable == 0 || !proof.can_apply(self, env, stack) {
            return;
        }
        match self.slot(s).cloned() {
            None => {
                let part = stack.take_portion(transferable);
                self.set_by_player(env, s, Some(part));
            }
            Some(mut current) if current.same_item_same_components(stack) => {
                *stack.count_mut() -= transferable;
                current.count += transferable;
                self.set_by_player(env, s, Some(current));
            }
            Some(_) => {}
        }
    }
}

impl Menu {
    fn observed_equipment_input(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        matches!(self.state.slots[s].role, Role::Armor(_))
            && env.player_entity_id.is_some()
            && env.can_equip.is_some_and(|parts| parts.listens(stack))
    }
    /// Cursor pickup/replacement uses one source scope, including its capacity check.
    pub(super) fn pickup_equip_carried(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        let Some(carried) = self.state.carried.as_ref() else {
            return false;
        };
        if !self.observed_equipment_input(env, s, carried) {
            return false;
        }
        let clicked = self.slot(s).cloned();
        if clicked.is_some() && !self.may_pickup(env, s) {
            return true;
        }
        let location = carried as *const _ as usize as u64;
        let mut input = self.state.carried.take();
        let mut source = EquipDecision::new(location, &mut input);
        if !env.can_equip.unwrap().prepare(
            &source,
            env.player_entity_id.unwrap() as u32,
            "minecraft:player",
        ) {
            self.state.carried = source.remove();
            return true;
        }
        let proof = self.equip_placement(env, s, &source);
        match clicked {
            None => {
                let amount = if primary { source.count } else { 1 };
                self.insert_from_source_with_placement(env, s, &mut source, amount, proof);
            }
            Some(clicked) if proof.allowed && clicked.same_item_same_components(&source) => {
                let amount = if primary { source.count } else { 1 };
                self.insert_from_source_with_placement(env, s, &mut source, amount, proof);
            }
            Some(clicked) if proof.allowed && source.count <= proof.max => {
                if proof.can_apply(self, env, &source) {
                    let carried = source.remove();
                    self.state.carried = Some(clicked);
                    self.set_by_player(env, s, carried);
                    return true;
                }
            }
            Some(clicked) if !proof.allowed && clicked.same_item_same_components(&source) => {
                let room = Self::max_stack(env, &source).saturating_sub(source.count);
                if let Some(mut taken) = self.try_remove(env, s, clicked.count, room) {
                    *source.count_mut().expect("pickup source retained") += taken.count;
                    self.on_take(env, s, &mut taken);
                }
            }
            Some(_) => {}
        }
        self.state.carried = source.remove();
        tidy(&mut self.state.carried);
        true
    }
    /// Number-key swaps have the same source ownership and single predicate ask.
    pub(super) fn swap_equip_source(&mut self, env: &Env, target: usize, inv_slot: usize) -> bool {
        let Some(stack) = self.state.containers[PLAYER][inv_slot].as_ref() else {
            return false;
        };
        if !self.observed_equipment_input(env, target, stack) {
            return false;
        }
        let taken = self.slot(target).cloned();
        if taken.is_some() && !self.may_pickup(env, target) {
            return true;
        }
        let location = stack as *const _ as usize as u64;
        let mut input = self.state.containers[PLAYER][inv_slot].take();
        let mut source = EquipDecision::new(location, &mut input);
        if !env.can_equip.unwrap().prepare(
            &source,
            env.player_entity_id.unwrap() as u32,
            "minecraft:player",
        ) {
            self.state.containers[PLAYER][inv_slot] = source.remove();
            return true;
        }
        let proof = self.equip_placement(env, target, &source);
        if !proof.can_apply(self, env, &source) {
            self.state.containers[PLAYER][inv_slot] = source.remove();
            return true;
        }
        if source.count > proof.max {
            let part = source.split(proof.max);
            self.state.containers[PLAYER][inv_slot] = source.remove();
            self.set_by_player(env, target, Some(part));
            if let Some(mut taken) = taken {
                self.on_take(env, target, &mut taken);
                if !self.inventory_add(env, &mut taken) {
                    self.drop_stack(&taken);
                }
            }
        } else {
            self.state.containers[PLAYER][inv_slot] = taken.clone();
            self.set_by_player(env, target, source.remove());
            if let Some(mut taken) = taken {
                self.on_take(env, target, &mut taken);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::equip::{CanEquip, EquipStamp, source_stamp};
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct FeetOnly {
        prepared: RefCell<Option<EquipStamp>>,
        preparations: Cell<usize>,
        asks: Cell<usize>,
    }
    impl CanEquip for FeetOnly {
        fn prepare(&self, stack: &ItemStack, _: u32, _: &str) -> bool {
            let stamp = source_stamp(stack).unwrap();
            let mut prepared = self.prepared.borrow_mut();
            if prepared.is_none() {
                *prepared = Some(stamp);
                self.preparations.set(self.preparations.get() + 1);
            }
            *prepared == Some(stamp)
        }
        fn can_apply(&self, stack: &ItemStack, _: u32) -> bool {
            *self.prepared.borrow() == source_stamp(stack)
        }
        fn can_equip(&self, stack: &ItemStack, slot: &str, _: u32, _: &str, _: bool) -> bool {
            assert_eq!(*self.prepared.borrow(), source_stamp(stack));
            self.asks.set(self.asks.get() + 1);
            slot == "feet"
        }
    }
    struct Unpreparable;
    impl CanEquip for Unpreparable {
        fn prepare(&self, _: &ItemStack, _: u32, _: &str) -> bool {
            false
        }
        fn can_equip(&self, _: &ItemStack, _: &str, _: u32, _: &str, _: bool) -> bool {
            panic!("unprepared equipment must refuse");
        }
    }
    #[test]
    #[ignore = "daily"]
    fn unpreparable_cursor_helmet_enters_inventory_but_not_head() {
        super::super::tests::with_menu(Kind::Inventory, |env, menu| {
            let env = Env {
                can_equip: Some(&Unpreparable),
                player_entity_id: Some(7),
                ..*env
            };
            menu.carried = Some(ItemStack::new("minecraft:iron_helmet", 3));
            menu.click(&env, 9, 0, Input::Pickup).unwrap();
            assert_eq!(menu.slot(9).unwrap().item.as_ref(), "minecraft:iron_helmet");
            assert_eq!(menu.slot(9).unwrap().count, 1);
            assert_eq!(menu.carried.as_ref().unwrap().count, 2);
            menu.carried = Some(ItemStack::new("minecraft:iron_helmet", 3));
            menu.click(&env, 5, 0, Input::Pickup).unwrap();
            assert!(menu.slot(5).is_none());
            assert_eq!(menu.carried.as_ref().unwrap().count, 3);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn four_slot_menu_transfer_prepares_once_and_stale_placement_refuses() {
        super::super::tests::with_menu(Kind::Inventory, |env, menu| {
            let parts = FeetOnly::default();
            let env = Env {
                can_equip: Some(&parts),
                player_entity_id: Some(7),
                ..*env
            };
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 3);
            assert!(menu.move_to_equip_source(&env, &mut helmet, 5, 9, false));
            assert_eq!((parts.preparations.get(), parts.asks.get()), (1, 4));
            assert_eq!(helmet.count, 2);
            assert_eq!(menu.slot(8).unwrap().count, 1);
            assert!((5..8).all(|slot| menu.slot(slot).is_none()));
            let parts = FeetOnly::default();
            let env = Env {
                can_equip: Some(&parts),
                ..env
            };
            let mut input = Some(helmet);
            let mut source = EquipDecision::new(42, &mut input);
            assert!(parts.prepare(&source, 7, "minecraft:player"));
            let proof = menu.equip_placement(&env, 8, &source);
            assert!(proof.can_apply(menu, &env, &source));
            menu.touch();
            assert!(!proof.can_apply(menu, &env, &source));
            let proof = menu.equip_placement(&env, 8, &source);
            *source.count_mut().unwrap() = 1;
            assert!(!proof.can_apply(menu, &env, &source));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn menus_without_actor_or_equipment_candidates_do_not_prepare() {
        super::super::tests::with_menu(Kind::Inventory, |env, menu| {
            let parts = FeetOnly::default();
            let env = Env {
                can_equip: Some(&parts),
                player_entity_id: None,
                ..*env
            };
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(menu.move_to_equip_source(&env, &mut helmet, 5, 6, false));
            assert_eq!(menu.slot(5).unwrap().item.as_ref(), "minecraft:iron_helmet");
            assert_eq!((parts.preparations.get(), parts.asks.get()), (0, 0));
            let env = Env {
                player_entity_id: Some(7),
                ..env
            };
            menu.carried = Some(ItemStack::new("minecraft:stone", 3));
            for (slot, button) in [(-999, 0), (9, 1), (10, 1), (-999, 2)] {
                menu.click(&env, slot, button, Input::QuickCraft).unwrap();
            }
            assert_eq!(menu.slot(9).unwrap().count, 1);
            assert_eq!(menu.slot(10).unwrap().count, 1);
            assert_eq!(menu.carried.as_ref().unwrap().count, 1);
            assert_eq!((parts.preparations.get(), parts.asks.get()), (0, 0));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn occupied_equipment_cursor_and_hotbar_swaps_ask_once_per_click() {
        for input in [Input::Pickup, Input::Swap] {
            super::super::tests::with_menu(Kind::Inventory, |env, menu| {
                let parts = FeetOnly::default();
                let env = Env {
                    can_equip: Some(&parts),
                    player_entity_id: Some(7),
                    ..*env
                };
                menu.set_slot(&env, 8, Some(ItemStack::new("minecraft:diamond_boots", 1)));
                if input == Input::Pickup {
                    menu.carried = Some(ItemStack::new("minecraft:iron_helmet", 1));
                } else {
                    menu.set_slot(&env, 36, Some(ItemStack::new("minecraft:iron_helmet", 1)));
                }
                menu.click(&env, 8, 0, input).unwrap();
                assert_eq!(menu.slot(8).unwrap().item.as_ref(), "minecraft:iron_helmet");
                let returned = if input == Input::Pickup {
                    menu.carried.as_ref()
                } else {
                    menu.slot(36)
                };
                assert_eq!(returned.unwrap().item.as_ref(), "minecraft:diamond_boots");
                assert_eq!((parts.preparations.get(), parts.asks.get()), (1, 1));
            });
        }
    }
}
