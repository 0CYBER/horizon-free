//! Local click, cursor, drag and pick-all algorithms.
use super::*;

impl Menu {
    pub(super) fn quick_replace(&self, s: usize, stack: &ItemStack, env: &Env) -> bool {
        match self.slot(s) {
            None => true,
            Some(have) if have.same_item_same_components(stack) => {
                have.count <= Self::max_stack(env, stack)
            }
            Some(_) => false,
        }
    }

    /// Whether a drag (quick craft) is in progress (`quickcraftStatus != 0`):
    /// the next click other than a quick craft only ends it.
    pub fn quick_crafting(&self) -> bool {
        self.state.quick_status != 0
    }

    pub(super) fn reset_quick_craft(&mut self) {
        self.touch();
        self.state.quick_status = 0;
        self.state.quick_slots.clear();
    }

    /// `AbstractContainerMenu.clicked`: Err carries a vanilla exception name.
    pub fn click(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
    ) -> Result<(), &'static str> {
        self.touch();
        let size = self.state.slots.len() as i32;
        let in_range = |i: i32| (0..size).contains(&i);
        if input == Input::QuickCraft {
            let expected = self.state.quick_status;
            self.state.quick_status = button & 3;
            if ((expected != 1 || self.state.quick_status != 2)
                && expected != self.state.quick_status)
                || self.state.carried.is_none()
            {
                self.reset_quick_craft();
            } else if self.state.quick_status == 0 {
                self.state.quick_type = (button >> 2) & 3;
                if self.state.quick_type == 0
                    || self.state.quick_type == 1
                    || (self.state.quick_type == 2 && self.state.creative)
                {
                    self.state.quick_status = 1;
                    self.state.quick_slots.clear();
                } else {
                    self.reset_quick_craft();
                }
            } else if self.state.quick_status == 1 {
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                let s = slot_index as usize;
                let carried = self.state.carried.clone().expect("checked");
                if self.quick_replace(s, &carried, env)
                    && self.may_place(env, s, &carried)
                    && (self.state.quick_type == 2
                        || carried.count as usize > self.state.quick_slots.len())
                    && self.can_drag_to(s)
                    && !self.state.quick_slots.contains(&s)
                {
                    self.state.quick_slots.push(s);
                }
            } else if self.state.quick_status == 2 {
                if !self.state.quick_slots.is_empty() {
                    if self.state.quick_slots.len() == 1 {
                        let s = self.state.quick_slots[0] as i32;
                        let ty = self.state.quick_type;
                        self.reset_quick_craft();
                        return self.click(env, s, ty, Input::Pickup);
                    }
                    let mut source = self.state.carried.clone().expect("checked");
                    let mut remaining = source.count as i64;
                    let observed = env.player_entity_id.is_some()
                        && env.can_equip.is_some_and(|parts| parts.listens(&source))
                        && self
                            .state
                            .quick_slots
                            .iter()
                            .any(|slot| matches!(self.state.slots[*slot].role, Role::Armor(_)));
                    let mut input = observed.then(|| Some(source.clone()));
                    let mut barrier = input
                        .as_mut()
                        .map(|slot| crate::mobs::equip::EquipDecision::new(0, slot));
                    let carried = barrier.as_deref().unwrap_or(&source);
                    if observed
                        && !env.can_equip.unwrap().prepare(
                            carried,
                            env.player_entity_id.unwrap() as u32,
                            "minecraft:player",
                        )
                    {
                        self.reset_quick_craft();
                        return Ok(());
                    }
                    for s in self.state.quick_slots.clone() {
                        let proof = self.equip_placement(env, s, carried);
                        if self.quick_replace(s, carried, env)
                            && proof.allowed
                            && (self.state.quick_type == 2
                                || carried.count as usize >= self.state.quick_slots.len())
                            && self.can_drag_to(s)
                        {
                            let have = self.slot(s).map_or(0, |c| c.count);
                            let max = Self::max_stack(env, &source).min(proof.max);
                            let new_count = quick_craft_count(
                                self.state.quick_type as u8,
                                source.count,
                                self.state.quick_slots.len(),
                                have,
                                max,
                                Self::max_stack(env, &source),
                            );
                            if !proof.can_apply(self, env, carried) {
                                continue;
                            }
                            remaining -= i64::from(new_count) - i64::from(have);
                            let mut placed = source.clone();
                            placed.count = new_count;
                            self.set_by_player(env, s, Some(placed));
                        }
                    }
                    self.state.carried = if let Some(barrier) = barrier.as_mut() {
                        *barrier.count_mut().expect("drag source retained") =
                            remaining.max(0) as u32;
                        barrier.remove()
                    } else {
                        source.count = remaining.max(0) as u32;
                        Some(source)
                    };
                    tidy(&mut self.state.carried);
                }
                self.reset_quick_craft();
            } else {
                self.reset_quick_craft();
            }
            return Ok(());
        }
        if self.state.quick_status != 0 {
            self.reset_quick_craft();
            return Ok(());
        }
        // A slot the mod's screen leaves out takes no click.
        if in_range(slot_index) && self.state.slots[slot_index as usize].role == Role::Hidden {
            return Ok(());
        }
        if matches!(input, Input::Pickup | Input::QuickMove) && (button == 0 || button == 1) {
            let primary = button == 0;
            if slot_index == -999 {
                if let Some(carried) = self.state.carried.as_mut() {
                    if primary {
                        let dropped = self.state.carried.take().expect("some");
                        self.drop_stack(&dropped);
                    } else {
                        let one = split(carried, 1);
                        self.drop_stack(&one);
                        tidy(&mut self.state.carried);
                    }
                }
            } else if input == Input::QuickMove {
                if slot_index < 0 {
                    return Ok(());
                }
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                let s = slot_index as usize;
                if !self.may_pickup(env, s) {
                    return Ok(());
                }
                let mut clicked = self.quick_move(env, s);
                while let Some(c) = &clicked {
                    if !self.slot(s).is_some_and(|now| same_item(now, c)) {
                        break;
                    }
                    clicked = self.quick_move(env, s);
                }
            } else {
                if slot_index < 0 {
                    return Ok(());
                }
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                self.pickup(env, slot_index as usize, primary);
            }
        } else if input == Input::Swap && ((0..9).contains(&button) || button == 40) {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            self.swap(env, slot_index as usize, button as usize);
        } else if input == Input::Clone
            && self.state.creative
            && self.state.carried.is_none()
            && slot_index >= 0
        {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            if let Some(stack) = self.slot(slot_index as usize) {
                let mut copy = stack.clone();
                copy.count = Self::max_stack(env, stack);
                // The cartography result's safeClone runs onCraftedBy.
                if self.state.slots[slot_index as usize].role == Role::CartoResult {
                    Self::map_crafted(env.maps, &mut copy);
                }
                self.state.carried = Some(copy);
            }
        } else if input == Input::Throw && self.state.carried.is_none() && slot_index >= 0 {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            let s = slot_index as usize;
            let amount = if button == 0 {
                1
            } else {
                self.slot(s).map_or(0, |c| c.count)
            };
            let mut taken = self.safe_take(env, s, amount, u32::MAX);
            if let Some(t) = &taken {
                self.drop_stack(t);
            }
            if button == 1 {
                while let Some(t) = &taken {
                    if !self.slot(s).is_some_and(|now| same_item(now, t)) {
                        break;
                    }
                    taken = self.safe_take(env, s, amount, u32::MAX);
                    if let Some(t) = &taken {
                        self.drop_stack(t);
                    }
                }
            }
        } else if input == Input::PickupAll && slot_index >= 0 {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            self.pickup_all(env, slot_index as usize, button);
        }
        Ok(())
    }

    /// The PICKUP branch of `doClick`: a bundle's override, else the
    /// plain click, ending with `slot.setChanged()`.
    pub(super) fn pickup(&mut self, env: &Env, s: usize, primary: bool) {
        self.touch();
        let mut canceled = false;
        let mut handled = false;
        if crate::native_client_hooks::has_mods() {
            let slot = self.state.slots[s];
            let mut carried = self.state.carried.take();
            let mut view = MenuSlotView {
                menu: self,
                env,
                slot: s,
            };
            let mut event = StackedOnOther {
                player: env.player,
                player_entity_id: env.player_entity_id,
                slot: s,
                container: slot.container,
                container_slot: slot.index,
                primary,
                carried: CarriedAccess(&mut carried),
                clicked: &mut view,
                cancellation_result: false,
            };
            native_item_stacked_on_other(env.loop_context, &mut event, &mut canceled);
            handled = event.cancellation_result;
            self.state.carried = carried;
        }
        let handled = if canceled {
            handled
        } else {
            self.bundle_click(env, s, primary)
        };
        if !handled {
            self.pickup_inner(env, s, primary);
        }
        self.slot_changed(env, s);
    }

    pub(super) fn pickup_inner(&mut self, env: &Env, s: usize, primary: bool) {
        self.touch();
        if self.pickup_equip_carried(env, s, primary) {
            return;
        }
        let clicked = self.slot(s).cloned();
        match clicked {
            None => {
                if let Some(mut carried) = self.state.carried.take() {
                    let amount = if primary { carried.count } else { 1 };
                    let mut input = Some(carried.clone());
                    self.safe_insert(env, s, &mut input, amount);
                    carried = input.unwrap_or_else(|| {
                        let mut e = carried.clone();
                        e.count = 0;
                        e
                    });
                    self.state.carried = Some(carried);
                    tidy(&mut self.state.carried);
                }
            }
            Some(clicked) if self.may_pickup(env, s) => match self.state.carried.take() {
                None => {
                    let amount = if primary {
                        clicked.count
                    } else {
                        clicked.count.div_ceil(2)
                    };
                    if let Some(mut taken) = self.try_remove(env, s, amount, u32::MAX) {
                        self.state.carried = Some(taken.clone());
                        self.on_take(env, s, &mut taken);
                        if self.state.slots[s].role == Role::CartoResult {
                            Self::map_crafted(env.maps, &mut taken);
                        }
                        if matches!(
                            self.state.slots[s].role,
                            Role::CartoResult | Role::CraftResult
                        ) {
                            self.state.carried = Some(taken);
                        }
                    }
                }
                Some(mut carried) => {
                    if self.may_place(env, s, &carried) {
                        if clicked.same_item_same_components(&carried) {
                            let amount = if primary { carried.count } else { 1 };
                            let mut input = Some(carried.clone());
                            self.safe_insert(env, s, &mut input, amount);
                            carried = input.unwrap_or_else(|| {
                                let mut e = carried.clone();
                                e.count = 0;
                                e
                            });
                            self.state.carried = Some(carried);
                            tidy(&mut self.state.carried);
                        } else if carried.count <= self.slot_max(env, s, &carried) {
                            self.state.carried = Some(clicked);
                            self.set_by_player(env, s, Some(carried));
                        } else {
                            self.state.carried = Some(carried);
                        }
                    } else if clicked.same_item_same_components(&carried) {
                        let room = Self::max_stack(env, &carried).saturating_sub(carried.count);
                        self.state.carried = Some(carried.clone());
                        if let Some(mut taken) = self.try_remove(env, s, clicked.count, room) {
                            if let Some(c) = self.state.carried.as_mut() {
                                c.count += taken.count;
                            }
                            self.on_take(env, s, &mut taken);
                            if self.state.slots[s].role == Role::CartoResult {
                                // onCraftedBy edits the taken stack, not the carried one.
                                Self::map_crafted(env.maps, &mut taken);
                            }
                        }
                    } else {
                        self.state.carried = Some(carried);
                    }
                }
            },
            Some(_) => {}
        }
    }

    /// The SWAP branch of `doClick`.
    pub(super) fn swap(&mut self, env: &Env, target: usize, button: usize) {
        self.touch();
        let inv_slot = button;
        let target_slot = self.state.slots[target];
        if target_slot.container == PLAYER && target_slot.index == inv_slot {
            return;
        }
        if self.swap_equip_source(env, target, inv_slot) {
            return;
        }
        let source = self.state.containers[PLAYER][inv_slot].clone();
        let target_stack = self.slot(target).cloned();
        if source.is_none() && target_stack.is_none() {
            return;
        }
        match (source, target_stack) {
            (None, Some(mut taken)) => {
                if self.may_pickup(env, target) {
                    self.state.containers[PLAYER][inv_slot] = Some(taken.clone());
                    self.set_by_player(env, target, None);
                    self.on_take(env, target, &mut taken);
                    if self.state.slots[target].role == Role::CraftResult {
                        self.state.containers[PLAYER][inv_slot] = Some(taken);
                    } else if self.state.slots[target].role == Role::CartoResult
                        && let Some(stack) = self.state.containers[PLAYER][inv_slot].as_mut()
                    {
                        Self::map_crafted(env.maps, stack);
                    }
                }
            }
            (Some(mut source), None) => {
                if self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.state.containers[PLAYER][inv_slot] = Some(source);
                        self.set_by_player(env, target, Some(part));
                    } else {
                        self.state.containers[PLAYER][inv_slot] = None;
                        self.set_by_player(env, target, Some(source));
                    }
                }
            }
            (Some(mut source), Some(mut taken)) => {
                if self.may_pickup(env, target) && self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.state.containers[PLAYER][inv_slot] = Some(source);
                        self.set_by_player(env, target, Some(part));
                        self.on_take(env, target, &mut taken);
                        if !self.inventory_add(env, &mut taken) {
                            self.drop_stack(&taken);
                        }
                    } else {
                        self.state.containers[PLAYER][inv_slot] = Some(taken.clone());
                        self.set_by_player(env, target, Some(source));
                        self.on_take(env, target, &mut taken);
                    }
                }
            }
            (None, None) => {}
        }
    }

    /// The PICKUP_ALL branch (double click).
    /// `canTakeItemForPickAll`: never the result slots of the menus that
    /// override it (crafting, smithing, stonecutter, cartography; a mod menu
    /// type's crafting result is one), MerchantMenu's or an item tab's
    /// display slots.
    /// The screen asks it before a double click's PICKUP_ALL
    /// (`AbstractContainerScreen.mouseReleased`): a refused slot's second
    /// click stays a PICKUP, another craft.
    pub fn can_take_for_pick_all(&self, t: usize) -> bool {
        self.state.kind != Kind::Merchant
            && self.state.slots[t].role != Role::CreativeDisplay
            && (self.state.slots[t].container != RESULT
                || matches!(self.state.kind, Kind::Loom | Kind::Grindstone | Kind::Anvil))
    }

    /// `AbstractContainerScreen.mouseReleased` on a Shift double click:
    /// QUICK_MOVE on every slot of `clicked`'s container that holds
    /// `moved`'s item (`lastQuickMoved`, the stack the first click moved).
    pub fn quick_move_all(
        &mut self,
        env: &Env,
        clicked: usize,
        moved: &ItemStack,
    ) -> Result<(), &'static str> {
        self.touch();
        self.quick_move_all_each(env, clicked, moved, &mut |_, _| {})
    }

    /// `quick_move_all`, calling `each` with the menu and the slot after each
    /// QUICK_MOVE (a server's client sends every one of them).
    pub fn quick_move_all_each(
        &mut self,
        env: &Env,
        clicked: usize,
        moved: &ItemStack,
        each: &mut dyn FnMut(&Self, usize),
    ) -> Result<(), &'static str> {
        self.touch();
        if clicked >= self.state.slots.len() || !self.can_take_for_pick_all(clicked) {
            return Ok(());
        }
        let container = self.state.slots[clicked].container;
        for t in 0..self.state.slots.len() {
            if self.may_pickup(env, t)
                && self.has_item(t)
                && self.state.slots[t].container == container
                && self.quick_replace(t, moved, env)
            {
                self.click(env, t as i32, 0, Input::QuickMove)?;
                each(self, t);
            }
        }
        Ok(())
    }

    /// `AbstractContainerScreen.shouldAddSlotToQuickCraft`: whether a drag
    /// of type `ty` over `dragged` slots so far takes slot `s` too.
    pub fn quick_craft_accepts(&self, env: &Env, s: usize, ty: i32, dragged: usize) -> bool {
        let Some(carried) = self.state.carried.as_ref() else {
            return false;
        };
        s < self.state.slots.len()
            && (carried.count as usize > dragged || ty == 2)
            && self.quick_replace(s, carried, env)
            && self.may_place(env, s, carried)
            && self.can_drag_to(s)
    }

    /// What a quick craft drag of type `ty` over `slots` shows
    /// (`AbstractContainerScreen.extractSlot` and
    /// `recalculateQuickCraftRemaining`): each slot's would-be count and
    /// whether the slot caps it (a yellow count), then the count left in
    /// hand. Nothing while the drag spans a single slot.
    pub fn quick_craft_preview(
        &self,
        env: &Env,
        ty: i32,
        slots: &[usize],
    ) -> Option<(Vec<Option<(u32, bool)>>, u32)> {
        let carried = self.state.carried.as_ref()?;
        if slots.len() < 2 {
            return None;
        }
        let carried_max = Self::max_stack(env, carried);
        // AbstractContainerMenu.getQuickCraftPlaceCount.
        let place = match ty {
            0 => carried.count / slots.len() as u32,
            1 => 1,
            2 => carried_max,
            _ => carried.count,
        };
        let mut left = i64::from(if ty == 2 { carried_max } else { carried.count });
        let mut shown = Vec::with_capacity(slots.len());
        for &s in slots {
            let carry = self.slot(s).map_or(0, |i| i.count);
            let max = carried_max.min(self.slot_max(env, s, carried));
            if ty != 2 {
                left -= i64::from((place + carry).min(max)) - i64::from(carry);
            }
            shown.push(
                (s < self.state.slots.len()
                    && self.quick_replace(s, carried, env)
                    && self.can_drag_to(s))
                .then(|| ((place + carry).min(max), place + carry > max)),
            );
        }
        Some((shown, left.max(0) as u32))
    }

    /// `canDragTo`: ItemPickerMenu keeps drags off its display slots.
    pub(super) fn can_drag_to(&self, s: usize) -> bool {
        self.state.slots[s].role != Role::CreativeDisplay
    }

    pub(super) fn pickup_all(&mut self, env: &Env, s: usize, button: i32) {
        self.touch();
        let Some(carried) = self.state.carried.clone() else {
            return;
        };
        if self.has_item(s) && self.may_pickup(env, s) {
            return;
        }
        let total = self.state.slots.len() as i32;
        let (start, step) = if button == 0 { (0, 1) } else { (total - 1, -1) };
        let carried_max = Self::max_stack(env, &carried);
        for pass in 0..2 {
            let mut i = start;
            while i >= 0
                && i < total
                && self.state.carried.as_ref().map_or(0, |c| c.count) < carried_max
            {
                let t = i as usize;
                i += step;
                let have = self.state.carried.clone().expect("some");
                if self.has_item(t)
                    && self.quick_replace(t, &have, env)
                    && self.may_pickup(env, t)
                    && self.can_take_for_pick_all(t)
                {
                    let item = self.slot(t).expect("has item").clone();
                    if pass != 0 || item.count != Self::max_stack(env, &item) {
                        let room = carried_max - have.count;
                        if let Some(removed) = self.safe_take(env, t, item.count, room)
                            && let Some(c) = self.state.carried.as_mut()
                        {
                            c.count += removed.count;
                        }
                    }
                }
            }
        }
    }
}
