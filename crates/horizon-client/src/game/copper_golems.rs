//! Copper golems on the game side: `TransportItemsBetweenContainers` (carry
//! items from copper chests to ordinary chests), weathering and turning into
//! statues, and the hand-over, waxing, scraping and shearing interactions.
use super::Game;
use horizon_client::{
    containers::BlockEntity,
    items::ItemStack,
    mobs::{Mob, Species, copper_golem as cg, goals::should_panic},
};
use horizon_core::random::RandomSource;
use horizon_world::chunk::BlockState;

type Key = (i32, i32, i32);
/// `TransportItemState`.
const TRAVELLING: i32 = 0;
const QUEUING: i32 = 1;
const INTERACTING: i32 = 2;
/// `ContainerInteractionState` (0 = none).
const PICKUP_ITEM: i32 = 1;
const PICKUP_NO_ITEM: i32 = 2;
const PLACE_ITEM: i32 = 3;
const PLACE_NO_ITEM: i32 = 4;
/// Positions that were unreachable are kept in `positions` with this offset.
const UNREACHABLE: i32 = 1 << 20;
/// `TransportItemsBetweenContainers.VISITED_POSITIONS_MEMORY_TIME`.
const MEMORY_TICKS: i32 = 6000;

/// Whether a block is a copper chest (the pick-up sources).
pub(super) fn is_copper_chest(name: &str) -> bool {
    name.ends_with("copper_chest")
}
/// The held stack (`getMainHandItem`).
pub(super) fn held(m: &Mob) -> Option<&ItemStack> {
    m.extra.items.first().and_then(Option::as_ref)
}
/// `setItemInHand(MAIN_HAND, ..)`, keeping the rendered copy in step.
pub(super) fn set_held(m: &mut Mob, stack: Option<ItemStack>) {
    let stack = stack.filter(|s| s.count > 0);
    m.extra.main_hand = stack.as_ref().map(|s| s.item.clone());
    m.extra.ints[1] = stack.as_ref().map_or(0, |s| s.count as i32);
    if m.extra.items.is_empty() {
        m.extra.items.push(None);
    }
    m.extra.items[0] = stack;
}
fn visited(m: &Mob) -> Vec<[i32; 3]> {
    m.extra
        .positions
        .iter()
        .filter(|p| p[1] < UNREACHABLE)
        .copied()
        .collect()
}
fn blocked_list(m: &Mob) -> impl Iterator<Item = [i32; 3]> + '_ {
    m.extra.positions.iter().map(|p| {
        if p[1] >= UNREACHABLE {
            [p[0], p[1] - UNREACHABLE, p[2]]
        } else {
            *p
        }
    })
}
fn chest_center(h: [i32; 3]) -> [f64; 3] {
    [
        f64::from(h[0]) + 0.5,
        f64::from(h[1]) + 0.5,
        f64::from(h[2]) + 0.5,
    ]
}
/// The copper golem state the model, sounds and animations follow.
fn set_state(m: &mut Mob, state: u8) {
    m.extra.variant3 = state;
}

impl Game<'_> {
    pub(super) fn tick_copper_golems(&mut self) {
        let ids: Vec<u32> = self
            .mobs
            .iter()
            .filter(|m| m.species() == Species::CopperGolem && m.alive() && !m.removed)
            .map(|m| m.id)
            .collect();
        for id in ids {
            let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
                continue;
            };
            let mut m = self.mobs[i].clone();
            self.copper_golem_step(&mut m);
            if let Some(i) = self.mobs.iter().position(|x| x.id == id) {
                self.mobs[i] = m;
            }
        }
        let alive: std::collections::HashSet<u32> = self.mobs.iter().map(|m| m.id).collect();
        self.golem_openers.retain(|id, _| alive.contains(id));
    }
    fn copper_golem_step(&mut self, m: &mut Mob) {
        if self.golem_weathering(m) {
            return;
        }
        // CountDownCooldownTicks(TRANSPORT_ITEMS_COOLDOWN_TICKS).
        if m.extra.scratch[3] > 0 {
            m.extra.scratch[3] -= 1;
        }
        // The visited positions fade after 6000 ticks.
        if m.extra.ints[2] > 0 && m.age as i32 >= m.extra.ints[2] {
            m.extra.positions.clear();
            m.extra.ints[2] = 0;
        }
        let running = m.extra.scratch[3] <= 0 && !should_panic(m) && m.extra.leash.is_none();
        if !running {
            if m.extra.home.is_some() || m.extra.scratch[0] != TRAVELLING {
                self.golem_on_start_travelling(m);
                m.extra.point = None;
                m.ai.nav.stop();
            }
            return;
        }
        let invalid = self.golem_update_invalid_target(m);
        if m.extra.home.is_none() {
            self.golem_on_start_travelling(m);
            return;
        }
        if invalid {
            return;
        }
        if m.extra.scratch[0] == QUEUING {
            self.golem_on_queuing(m);
        }
        if m.extra.scratch[0] == TRAVELLING {
            self.golem_on_travel(m);
        }
        if m.extra.scratch[0] == INTERACTING {
            self.golem_on_reached(m);
        }
    }
    // ---- weathering -------------------------------------------------------
    /// `CopperGolem.updateWeathering`; true when it turned into a statue.
    fn golem_weathering(&mut self, m: &mut Mob) -> bool {
        let now = self.world.game_time;
        let next = m.extra.vector[0] as i64;
        if next == -2 {
            return false;
        }
        // updateWeathering draws from the level random.
        let r = &mut self.world.random;
        let mut interval = || 504_000 + i64::from(r.next_int_bound(48_001).unwrap_or(0));
        if next == -1 {
            m.extra.vector[0] = (now + interval()) as f64;
            return false;
        }
        let fully = m.extra.variant >= 3;
        if now >= next && !fully {
            let new = cg::weather_next(m.extra.variant);
            m.extra.variant = new;
            m.extra.vector[0] = if new >= 3 {
                0.0
            } else {
                (next + interval()) as f64
            };
        }
        if fully && self.golem_can_turn_to_statue(m) {
            self.golem_turn_to_statue(m);
            return true;
        }
        false
    }
    fn golem_can_turn_to_statue(&mut self, m: &mut Mob) -> bool {
        let p = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        let at = self.world.block(p[0], p[1], p[2]);
        self.res.facts.is_air(at) && self.world.random.next_float() <= 0.0058
    }
    fn golem_turn_to_statue(&mut self, m: &mut Mob) {
        let pos = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        // The pose comes from the golem's own random (`this.random.nextInt(0, 4)`).
        let pose = ["standing", "sitting", "running", "star"]
            [m.random.next_int_bound(4).unwrap_or(0) as usize];
        // Direction.fromYRot.
        let facing = ["south", "west", "north", "east"]
            [((f64::from(m.yaw) / 90.0 + 0.5).floor() as i32 & 3) as usize];
        let state = BlockState {
            name: "minecraft:oxidized_copper_golem_statue".into(),
            properties: [
                ("facing".to_string(), facing.to_string()),
                ("copper_golem_pose".to_string(), pose.to_string()),
                ("waterlogged".to_string(), "false".to_string()),
            ]
            .into(),
        };
        let registry = self.res.assets.registry;
        let Some(id) = registry.id(&state) else {
            return;
        };
        if self.level_set(pos, id).is_err() {
            return;
        }
        self.golem_drop_equipment(m);
        self.golem_release(m);
        if m.extra.leash.is_some() {
            self.break_leash(m.id, true, false);
            m.extra.leash = None;
        }
        m.removed = true;
        self.audio.play(
            "minecraft:entity.copper_golem_become_statue",
            Some([m.position.x, m.position.y, m.position.z]),
            1.0,
            1.0,
        );
    }
    /// `Mob.dropPreservedEquipment`: the guaranteed drops in `EquipmentSlot`
    /// order, the carried stack (main hand) then the antenna flower (saddle),
    /// each `spawnAtLocation` at the golem's feet.
    pub(super) fn golem_drop_equipment(&mut self, m: &mut Mob) {
        let at = m.position;
        if let Some(stack) = held(m).cloned() {
            set_held(m, None);
            self.spawn_at_location(at, 0.0, stack);
        }
        if let Some(antenna) = m.saddle_item() {
            m.set_saddle(None);
            self.spawn_at_location(at, 0.0, antenna);
        }
    }
    // ---- containers -------------------------------------------------------
    /// The block entities of the container at a chest, in slot order.
    fn golem_container(&self, key: Key) -> Vec<Key> {
        let Some(other) = self.chest_other_half(key) else {
            return vec![key];
        };
        let right = self
            .res
            .assets
            .registry
            .state(self.world.block(key.0, key.1, key.2))
            .and_then(|s| s.properties.get("type"))
            .is_some_and(|t| t == "right");
        if right {
            vec![key, other]
        } else {
            vec![other, key]
        }
    }
    fn golem_block_name(&self, key: Key) -> std::sync::Arc<str> {
        self.res
            .facts
            .get(self.world.block(key.0, key.1, key.2))
            .name
            .clone()
    }
    fn golem_chest_blocked(&self, keys: &[Key]) -> bool {
        keys.iter().any(|k| {
            let above = self.world.block(k.0, k.1 + 1, k.2);
            self.res.behaviours.facts(above).flags
                & horizon_world::level::data::state_flags::CONDUCTOR
                != 0
        })
    }
    fn golem_slots(&self, keys: &[Key]) -> Vec<Option<ItemStack>> {
        keys.iter()
            .filter_map(|k| self.world.block_entities.get(k))
            .flat_map(|e| e.slots().to_vec())
            .collect()
    }
    fn golem_slot_mut(&mut self, keys: &[Key], mut n: usize) -> Option<&mut Option<ItemStack>> {
        for k in keys {
            let len = self.world.block_entities.get(k)?.slots().len();
            if n < len {
                return self.world.block_entities.get_mut(k)?.slots_mut().get_mut(n);
            }
            n -= len;
        }
        None
    }
    fn golem_max_stack(&self, stack: &ItemStack) -> u32 {
        self.res
            .items
            .get(&stack.item)
            .map_or(64, |i| i.max_stack)
            .max(1)
    }
    /// `pickupItemFromContainer`: up to 16 of the first non-empty slot.
    fn golem_pick_up(&mut self, keys: &[Key]) -> Option<ItemStack> {
        let slots = self.golem_slots(keys);
        let (n, stack) = slots
            .iter()
            .enumerate()
            .find_map(|(n, s)| s.as_ref().map(|s| (n, s.clone())))?;
        let take = stack.count.min(16);
        let slot = self.golem_slot_mut(keys, n)?;
        let mut out = stack;
        out.count = take;
        match slot {
            Some(s) if s.count > take => s.count -= take,
            _ => *slot = None,
        }
        Some(out)
    }
    /// `addItemsToContainer`: the part that did not fit.
    fn golem_put_down(&mut self, keys: &[Key], mut stack: ItemStack) -> Option<ItemStack> {
        let max = self.golem_max_stack(&stack);
        let slots = self.golem_slots(keys);
        for (n, existing) in slots.iter().enumerate() {
            match existing {
                None => {
                    if let Some(slot) = self.golem_slot_mut(keys, n) {
                        *slot = Some(stack);
                    }
                    return None;
                }
                Some(e)
                    if e.item == stack.item
                        && e.damage == stack.damage
                        && e.extra == stack.extra
                        && e.count < max =>
                {
                    let room = max - e.count;
                    let add = room.min(stack.count);
                    if let Some(Some(s)) = self.golem_slot_mut(keys, n) {
                        s.count += add;
                    }
                    // Vanilla subtracts the whole room, which empties the stack
                    // when it fitted.
                    stack.count = stack.count.saturating_sub(room);
                    if stack.count == 0 {
                        return None;
                    }
                }
                _ => {}
            }
        }
        Some(stack)
    }
    /// Whether any other golem or a player has the chest open.
    fn golem_chest_busy(&self, m: &Mob, keys: &[Key]) -> bool {
        self.golem_openers
            .iter()
            .any(|(id, k)| *id != m.id && keys.contains(k))
            || self.container.is_some_and(|c| keys.contains(&c))
    }
    // ---- the behaviour ----------------------------------------------------
    fn golem_wanted_block(&self, m: &Mob, key: Key) -> bool {
        let name = self.golem_block_name(key);
        if held(m).is_none() {
            is_copper_chest(&name)
        } else {
            matches!(&*name, "minecraft:chest" | "minecraft:trapped_chest")
        }
    }
    fn golem_update_invalid_target(&mut self, m: &mut Mob) -> bool {
        if self.golem_has_valid_target(m) {
            return false;
        }
        self.golem_stop_targeting(m);
        match self.golem_find_target(m) {
            Some(t) => {
                m.extra.home = Some([t.0, t.1, t.2]);
                self.golem_on_start_travelling(m);
                self.golem_set_visited(m, [t.0, t.1, t.2]);
            }
            None => self.golem_enter_cooldown(m),
        }
        true
    }
    fn golem_has_valid_target(&mut self, m: &mut Mob) -> bool {
        let Some(h) = m.extra.home else { return false };
        let key = (h[0], h[1], h[2]);
        let valid = self.golem_wanted_block(m, key)
            && matches!(
                self.world.block_entities.get(&key),
                Some(BlockEntity::Chest(_))
            )
            && !self.golem_chest_blocked(&self.golem_container(key));
        if !valid {
            return false;
        }
        if m.extra.scratch[0] != TRAVELLING {
            return true;
        }
        if self.golem_valid_travelling_path(m, h) {
            return true;
        }
        self.golem_mark_unreachable(m, h);
        false
    }
    /// `hasValidTravellingPath`, without the line-of-sight clip.
    fn golem_valid_travelling_path(&mut self, m: &mut Mob, h: [i32; 3]) -> bool {
        let w = self.mob_world_base(&[]);
        let path = match &m.ai.nav.path {
            Some(p) => Some(p.clone()),
            None => m.create_path_to(&w, h, 0),
        };
        let from = match path.as_ref().and_then(|p| p.end()) {
            Some(n) => [
                f64::from(n.x) + 0.5,
                f64::from(n.y) + m.kind.height / 2.0,
                f64::from(n.z) + 0.5,
            ],
            None => [
                m.position.x,
                m.position.y + m.kind.height / 2.0,
                m.position.z,
            ],
        };
        let reach = golem_within(m, self.golem_range(m), from, h);
        (path.is_none() && !reach) || reach
    }
    fn golem_range(&self, m: &Mob) -> f64 {
        if m.ai.nav.path.as_ref().is_some_and(|p| p.is_done()) {
            1.0
        } else {
            0.5
        }
    }
    fn golem_find_target(&self, m: &Mob) -> Option<Key> {
        let bx = m.position.x.floor() as i32;
        let by = m.position.y.floor() as i32;
        let bz = m.position.z.floor() as i32;
        let skip: Vec<[i32; 3]> = blocked_list(m).collect();
        let mut best: Option<(f64, Key)> = None;
        for (key, entity) in &self.world.block_entities {
            if !matches!(entity, BlockEntity::Chest(_)) {
                continue;
            }
            if (key.0 - bx).abs() > 32 || (key.1 - by).abs() > 8 || (key.2 - bz).abs() > 32 {
                continue;
            }
            let center = chest_center([key.0, key.1, key.2]);
            let d = (center[0] - m.position.x).powi(2)
                + (center[1] - m.position.y).powi(2)
                + (center[2] - m.position.z).powi(2);
            if best.is_some_and(|(b, _)| d >= b) {
                continue;
            }
            if !self.golem_wanted_block(m, *key) {
                continue;
            }
            let keys = self.golem_container(*key);
            if keys.iter().any(|k| skip.contains(&[k.0, k.1, k.2])) {
                continue;
            }
            if self.golem_chest_blocked(&keys) {
                continue;
            }
            best = Some((d, *key));
        }
        best.map(|b| b.1)
    }
    fn golem_set_visited(&mut self, m: &mut Mob, h: [i32; 3]) {
        let mut list = visited(m);
        if !list.contains(&h) {
            list.push(h);
        }
        if list.len() > 10 {
            self.golem_enter_cooldown(m);
            return;
        }
        m.extra.positions.retain(|p| p[1] >= UNREACHABLE);
        m.extra.positions.extend(list);
        m.extra.ints[2] = m.age as i32 + MEMORY_TICKS;
    }
    fn golem_mark_unreachable(&mut self, m: &mut Mob, h: [i32; 3]) {
        m.extra.positions.retain(|p| *p != h);
        let marked = [h[0], h[1] + UNREACHABLE, h[2]];
        if !m.extra.positions.contains(&marked) {
            m.extra.positions.push(marked);
        }
        let count = m
            .extra
            .positions
            .iter()
            .filter(|p| p[1] >= UNREACHABLE)
            .count();
        if count > 50 {
            self.golem_enter_cooldown(m);
        } else {
            m.extra.ints[2] = m.age as i32 + MEMORY_TICKS;
        }
    }
    fn golem_stop_targeting(&mut self, m: &mut Mob) {
        m.extra.scratch[1] = 0;
        m.extra.home = None;
        m.extra.point = None;
        m.ai.nav.stop();
    }
    fn golem_clear_memories(&mut self, m: &mut Mob) {
        self.golem_stop_targeting(m);
        m.extra.positions.clear();
    }
    fn golem_enter_cooldown(&mut self, m: &mut Mob) {
        self.golem_stop_targeting(m);
        m.extra.scratch[3] = 140;
        m.extra.positions.clear();
    }
    /// `onStartTravelling` and the golem's own consumer.
    fn golem_on_start_travelling(&mut self, m: &mut Mob) {
        self.golem_release(m);
        set_state(m, cg::IDLE);
        m.extra.scratch[0] = TRAVELLING;
        m.extra.scratch[2] = 0;
        m.extra.scratch[1] = 0;
    }
    /// `clearOpenedChestPos` (and so `stopOpen` at the next recheck).
    fn golem_release(&mut self, m: &Mob) {
        if let Some(key) = self.golem_openers.remove(&m.id) {
            let keys = self.golem_container(key);
            if !self.golem_chest_busy(m, &keys) {
                self.golem_chest_sound(key, false);
            }
        }
    }
    fn golem_chest_sound(&mut self, key: Key, open: bool) {
        let name = self.golem_block_name(key);
        let base = name
            .trim_start_matches("minecraft:")
            .trim_start_matches("waxed_");
        let kind = if base.starts_with("weathered") {
            "copper_chest_weathered"
        } else if base.starts_with("oxidized") {
            "copper_chest_oxidized"
        } else if base.ends_with("copper_chest") {
            "copper_chest"
        } else {
            "chest"
        };
        let side = if open { "open" } else { "close" };
        let at = chest_center([key.0, key.1, key.2]);
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        self.audio.play(
            &format!("minecraft:block.{kind}.{side}"),
            Some(at),
            0.5,
            pitch,
        );
    }
    fn golem_on_queuing(&mut self, m: &mut Mob) {
        let Some(h) = m.extra.home else { return };
        let keys = self.golem_container((h[0], h[1], h[2]));
        if !self.golem_chest_busy(m, &keys) {
            m.extra.scratch[0] = TRAVELLING;
            m.extra.point = Some(chest_center(h));
        }
    }
    fn golem_on_travel(&mut self, m: &mut Mob) {
        let Some(h) = m.extra.home else { return };
        let key = (h[0], h[1], h[2]);
        let keys = self.golem_container(key);
        let center = [
            m.position.x,
            m.position.y + m.kind.height / 2.0,
            m.position.z,
        ];
        if golem_within(m, 3.0, center, h) && self.golem_chest_busy(m, &keys) {
            // startQueuing.
            m.extra.point = None;
            m.ai.nav.stop();
            m.extra.scratch[0] = QUEUING;
        } else if golem_within(m, self.golem_range(m), center, h) {
            // startOnReachedTargetInteraction.
            let slots = self.golem_slots(&keys);
            let state = if held(m).is_none() {
                if slots.iter().any(Option::is_some) {
                    PICKUP_ITEM
                } else {
                    PICKUP_NO_ITEM
                }
            } else if slots.iter().all(Option::is_none)
                || slots
                    .iter()
                    .flatten()
                    .any(|s| held(m).is_some_and(|h| h.item == s.item))
            {
                PLACE_ITEM
            } else {
                PLACE_NO_ITEM
            };
            m.extra.scratch[2] = state;
            m.extra.scratch[0] = INTERACTING;
        } else {
            m.extra.point = Some(chest_center(h));
        }
    }
    fn golem_on_reached(&mut self, m: &mut Mob) {
        let Some(h) = m.extra.home else { return };
        let key = (h[0], h[1], h[2]);
        let center = [
            m.position.x,
            m.position.y + m.kind.height / 2.0,
            m.position.z,
        ];
        if !golem_within(m, 2.0, center, h) {
            self.golem_on_start_travelling(m);
            return;
        }
        m.extra.scratch[1] += 1;
        let ticks = m.extra.scratch[1];
        // onTargetInteraction: stand still, and the reached interaction.
        m.extra.point = None;
        m.ai.nav.stop();
        let (state, sound) = match m.extra.scratch[2] {
            PICKUP_ITEM => (cg::GETTING_ITEM, Some("entity.copper_golem.no_item_get")),
            PICKUP_NO_ITEM => (
                cg::GETTING_NO_ITEM,
                Some("entity.copper_golem.no_item_no_get"),
            ),
            PLACE_ITEM => (cg::DROPPING_ITEM, Some("entity.copper_golem.item_drop")),
            PLACE_NO_ITEM => (
                cg::DROPPING_NO_ITEM,
                Some("entity.copper_golem.item_no_drop"),
            ),
            _ => (cg::IDLE, None),
        };
        if m.extra.scratch[2] != 0 {
            if ticks == 1 {
                let keys = self.golem_container(key);
                if !self.golem_chest_busy(m, &keys) {
                    self.golem_chest_sound(key, true);
                }
                self.golem_openers.insert(m.id, key);
                set_state(m, state);
            }
            if ticks == 9
                && let Some(sound) = sound
            {
                let at = [m.position.x, m.position.y, m.position.z];
                self.audio
                    .play(&format!("minecraft:{sound}"), Some(at), 1.0, 1.0);
            }
            if ticks == 60 {
                self.golem_release(m);
            }
        }
        if ticks >= 60 {
            let keys = self.golem_container(key);
            if held(m).is_none() {
                if self.golem_slots(&keys).iter().any(Option::is_some) {
                    // pickUpItems.
                    let stack = self.golem_pick_up(&keys);
                    set_held(m, stack);
                    self.golem_clear_memories(m);
                } else {
                    self.golem_stop_targeting(m);
                }
            } else if self.golem_slots(&keys).iter().all(Option::is_none)
                || self
                    .golem_slots(&keys)
                    .iter()
                    .flatten()
                    .any(|s| held(m).is_some_and(|h| h.item == s.item))
            {
                // putDownItem.
                let stack = held(m)
                    .cloned()
                    .unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
                let left = self.golem_put_down(&keys, stack);
                let emptied = left.is_none();
                set_held(m, left);
                if emptied {
                    self.golem_clear_memories(m);
                } else {
                    self.golem_stop_targeting(m);
                }
            } else {
                self.golem_stop_targeting(m);
            }
            self.golem_on_start_travelling(m);
        }
    }
    // ---- interactions -----------------------------------------------------
    /// `CopperGolem.mobInteract`; true when handled.
    pub(super) fn interact_copper_golem(&mut self, index: usize, item: &str) -> bool {
        if self.mobs[index].species() != Species::CopperGolem || !self.mobs[index].alive() {
            return false;
        }
        let at = {
            let m = &self.mobs[index];
            [m.position.x, m.position.y, m.position.z]
        };
        if item.is_empty() {
            // Empty hand: the golem throws what it carries back.
            let Some(h) = held(&self.mobs[index]).cloned() else {
                return false;
            };
            set_held(&mut self.mobs[index], None);
            // BehaviorUtils.throwItem: from 0.3 below the golem's eyes towards
            // the player's feet at 0.3 blocks a tick, default pickup delay.
            let p = self.player.as_ref().map(|p| p.position);
            let eye = self.mobs[index].eye_height();
            let from = [at[0], at[1] + eye - f64::from(0.3f32), at[2]];
            let v = p.map_or([0.0; 3], |p| {
                let d = [p.x - at[0], p.y - at[1], p.z - at[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len < 1.0e-5 {
                    return [0.0; 3];
                }
                let k = f64::from(0.3f32);
                [d[0] / len * k, d[1] / len * k, d[2] / len * k]
            });
            self.spawn_item(h, from, v, 10);
            return true;
        }
        let name = item;
        let is_axe = name.ends_with("_axe");
        let waxed = self.mobs[index].extra.vector[0] as i64 == -2;
        if name == "minecraft:shears" && self.golem_shearable(index) {
            // CopperGolem.shear: the sound, the antenna slot emptied (an
            // unequip) and the flower dropped 1.5 up.
            self.audio
                .play("minecraft:entity.copper_golem.shear", Some(at), 1.0, 1.0);
            let antenna = self.mobs[index].saddle_item();
            self.mobs[index].set_saddle(None);
            let id = self.mobs[index].id;
            let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
            self.post_game_event("minecraft:unequip", at, ctx);
            if let Some(a) = antenna {
                let p = self.mobs[index].position;
                self.spawn_at_location(p, 1.5, a);
            }
            self.damage_held_tool();
            return true;
        }
        if name == "minecraft:honeycomb" && !waxed {
            self.mobs[index].extra.vector[0] = -2.0;
            self.audio
                .play("minecraft:item.honeycomb.wax_on", Some(at), 1.0, 1.0);
            self.consume_held();
            return true;
        }
        if is_axe && waxed {
            self.mobs[index].extra.vector[0] = -1.0;
            self.audio
                .play("minecraft:item.axe.scrape", Some(at), 1.0, 1.0);
            self.damage_held_tool();
            return true;
        }
        if is_axe && self.mobs[index].extra.variant > 0 {
            let m = &mut self.mobs[index];
            m.extra.vector[0] = -1.0;
            m.extra.variant = cg::weather_previous(m.extra.variant);
            self.audio
                .play("minecraft:item.axe.scrape", Some(at), 1.0, 1.0);
            self.damage_held_tool();
            return true;
        }
        false
    }
    /// `readyForShearing`: the antenna holds a flower shears take.
    pub(super) fn golem_shearable(&self, index: usize) -> bool {
        self.mobs[index].saddle_item().is_some_and(|a| {
            self.res
                .items
                .item_tags
                .get("minecraft:shearable_from_copper_golem")
                .is_some_and(|t| t.contains(&*a.item))
        })
    }
}
/// `isWithinTargetDistance`: the golem's box centred on `from` against the
/// chest's collision box inflated by `distance` (and half a block vertically).
fn golem_within(m: &Mob, distance: f64, from: [f64; 3], h: [i32; 3]) -> bool {
    let hw = m.kind.width / 2.0;
    let hh = m.kind.height / 2.0;
    let lo = [
        f64::from(h[0]) + 0.0625 - distance,
        f64::from(h[1]) - 0.5,
        f64::from(h[2]) + 0.0625 - distance,
    ];
    let hi = [
        f64::from(h[0]) + 0.9375 + distance,
        f64::from(h[1]) + 0.875 + 0.5,
        f64::from(h[2]) + 0.9375 + distance,
    ];
    from[0] + hw > lo[0]
        && from[0] - hw < hi[0]
        && from[1] + hh > lo[1]
        && from[1] - hh < hi[1]
        && from[2] + hw > lo[2]
        && from[2] - hw < hi[2]
}
