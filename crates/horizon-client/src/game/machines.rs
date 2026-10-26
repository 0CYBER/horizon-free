//! Item-moving blocks: `HopperBlockEntity` (push, pull, item entities,
//! transfer cooldowns), `DispenserBlock` / `DropperBlock.dispenseFrom` with
//! the vanilla dispense behaviours Horizon has entities for, and container
//! comparator signals (`AbstractContainerMenu.getRedstoneSignalFromContainer`).
use super::Game;

mod crafter;
use horizon_client::{
    Result,
    containers::{BlockEntity, HOPPER_SLOTS},
    items::{ItemStack, Items},
    world::World,
};
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};
use horizon_world::{level::Behaviours, registry::BlockStateId};

type Pos = (i32, i32, i32);

/// `HopperBlockEntity` transient state (`TransferCooldown`, `tickedGameTime`).
#[derive(Clone, Copy, Debug)]
pub(super) struct HopperState {
    cooldown: i32,
    ticked: i64,
}
impl Default for HopperState {
    fn default() -> Self {
        Self {
            cooldown: -1,
            ticked: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Generic,
    /// `AbstractFurnaceBlockEntity` (a `WorldlyContainer`).
    Furnace,
    Hopper,
    /// `CrafterBlockEntity`: disabled slots and the smallest-stack-first rule.
    Crafter,
    /// `BrewingStandBlockEntity` (a `WorldlyContainer`).
    Brewing,
    /// `ComposterBlock.getContainer` at this level (a `WorldlyContainerHolder`
    /// without a block entity): input below 7, empty at 7, bone meal at 8.
    Composter(i32),
}

/// A container: one block entity, or a double chest (`CompoundContainer`).
#[derive(Clone, Debug)]
struct View {
    parts: Vec<(Pos, usize)>,
    kind: Kind,
}
impl View {
    fn len(&self) -> usize {
        self.parts.iter().map(|p| p.1).sum()
    }
    fn locate(&self, mut slot: usize) -> Option<(Pos, usize)> {
        for &(pos, size) in &self.parts {
            if slot < size {
                return Some((pos, slot));
            }
            slot -= size;
        }
        None
    }
    /// `WorldlyContainer.getSlotsForFace` or every slot.
    fn slots_for_face(&self, face: Option<Direction>) -> Vec<usize> {
        match (self.kind, face) {
            (Kind::Furnace, Some(Direction::Down)) => vec![2, 1],
            (Kind::Furnace, Some(Direction::Up)) => vec![0],
            (Kind::Furnace, Some(_)) => vec![1],
            (Kind::Brewing, Some(Direction::Up)) => vec![3],
            (Kind::Brewing, Some(Direction::Down)) => vec![0, 1, 2, 3],
            (Kind::Brewing, Some(_)) => vec![0, 1, 2, 4],
            (Kind::Composter(8), Some(Direction::Down)) => vec![0],
            (Kind::Composter(level), Some(Direction::Up)) if level < 7 => vec![0],
            (Kind::Composter(_), _) => vec![],
            _ => (0..self.len()).collect(),
        }
    }
}

fn dir_from_name(name: &str) -> Option<Direction> {
    Some(match name {
        "down" => Direction::Down,
        "up" => Direction::Up,
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        "east" => Direction::East,
        _ => return None,
    })
}
fn step(d: Direction) -> [i32; 3] {
    match d {
        Direction::Down => [0, -1, 0],
        Direction::Up => [0, 1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::West => [-1, 0, 0],
        Direction::East => [1, 0, 0],
    }
}
fn offset(p: Pos, d: Direction) -> Pos {
    let s = step(d);
    (p.0 + s[0], p.1 + s[1], p.2 + s[2])
}
fn clockwise(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        other => other,
    }
}

/// State-level container queries shared by the game and comparator signals.
struct Containers<'a> {
    world: &'a World,
    behaviours: &'a Behaviours,
    items: &'a Items,
    registry: &'static horizon_world::registry::BlockRegistry,
    /// Brewing recipes, for what a brewing stand accepts (not needed for signals).
    brewing: Option<&'a horizon_client::brewing::Brewing>,
    /// The stack of a full composter's `OutputContainer`.
    bone_meal: ItemStack,
}
impl Containers<'_> {
    fn prop(&self, state: BlockStateId, key: &str) -> Option<&str> {
        self.registry
            .state(state)?
            .properties
            .get(key)
            .map(String::as_str)
    }
    fn name(&self, state: BlockStateId) -> &str {
        self.registry.state(state).map_or("", |s| s.name.as_str())
    }
    fn max_stack(&self, stack: &ItemStack) -> u32 {
        self.items
            .get(&stack.item)
            .map_or(64, |i| i.max_stack)
            .max(1)
    }
    /// `HopperBlockEntity.getBlockContainer` (double chests combined, right half
    /// first). `blocked_chests`: `ChestBlock.getContainer(.., false)` returns
    /// nothing for a chest under a redstone conductor.
    fn view(&self, pos: Pos, blocked_chests: bool) -> Option<View> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let name = self.name(state);
        if name == "minecraft:composter" {
            let level = self.prop(state, "level").and_then(|l| l.parse().ok())?;
            return Some(View {
                parts: vec![(pos, usize::from(level != 7))],
                kind: Kind::Composter(level),
            });
        }
        let entity = self.world.block_entities.get(&pos)?;
        // A beehive keeps bees, not items: it is no `Container`.
        if matches!(entity, BlockEntity::Beehive(_)) {
            return None;
        }
        let kind = match (entity, name) {
            (BlockEntity::Furnace(_), _) => Kind::Furnace,
            (BlockEntity::Crafter(_), _) => Kind::Crafter,
            (BlockEntity::Brewing(_), _) => Kind::Brewing,
            (_, "minecraft:hopper") => Kind::Hopper,
            _ => Kind::Generic,
        };
        let size = entity.slots().len();
        if matches!(name, "minecraft:chest" | "minecraft:trapped_chest") {
            let is_blocked = |p: Pos| {
                let above = self.world.block(p.0, p.1 + 1, p.2);
                self.behaviours.facts(above).flags
                    & horizon_world::level::data::state_flags::CONDUCTOR
                    != 0
            };
            if blocked_chests && is_blocked(pos) {
                return None;
            }
            let kind_prop = self.prop(state, "type").unwrap_or("single");
            if kind_prop != "single"
                && let Some(facing) = self.prop(state, "facing").and_then(dir_from_name)
            {
                // ChestBlock.getConnectedDirection
                let connected = if kind_prop == "left" {
                    clockwise(facing)
                } else {
                    clockwise(clockwise(clockwise(facing)))
                };
                let other = offset(pos, connected);
                let other_state = self.world.block(other.0, other.1, other.2);
                let paired = self.name(other_state) == name
                    && self
                        .prop(other_state, "type")
                        .is_some_and(|t| t != "single" && t != kind_prop)
                    && self
                        .world
                        .block_entities
                        .get(&other)
                        .is_some_and(|e| e.slots().len() == 27);
                if paired {
                    if blocked_chests && is_blocked(other) {
                        return None;
                    }
                    let parts = if kind_prop == "right" {
                        vec![(pos, size), (other, 27)]
                    } else {
                        vec![(other, 27), (pos, size)]
                    };
                    return Some(View { parts, kind });
                }
            }
        }
        Some(View {
            parts: vec![(pos, size)],
            kind,
        })
    }
    fn get(&self, view: &View, slot: usize) -> Option<&ItemStack> {
        if let Kind::Composter(level) = view.kind {
            // OutputContainer holds one bone meal; InputContainer composts at once.
            return (level == 8 && slot == 0).then_some(&self.bone_meal);
        }
        let (pos, i) = view.locate(slot)?;
        self.world
            .block_entities
            .get(&pos)?
            .slots()
            .get(i)?
            .as_ref()
    }
    fn is_empty(&self, view: &View) -> bool {
        (0..view.len()).all(|i| self.get(view, i).is_none())
    }
    /// `Container.canPlaceItem` (+ `canPlaceItemThroughFace` for furnaces).
    fn can_place(&self, view: &View, slot: usize, stack: &ItemStack) -> bool {
        if view.kind == Kind::Crafter {
            return match self.world.block_entities.get(&view.parts[0].0) {
                Some(BlockEntity::Crafter(c)) => {
                    // The slot's own stack sets the limit (`slotStack.getMaxStackSize`).
                    let max = c.items.get(slot).and_then(Option::as_ref).unwrap_or(stack);
                    c.can_place(slot, stack, self.max_stack(max))
                }
                _ => false,
            };
        }
        if view.kind == Kind::Brewing {
            // BrewingStandBlockEntity.canPlaceItem
            return match slot {
                4 => self
                    .items
                    .get(&stack.item)
                    .is_some_and(|i| i.brewing_fuel.is_some()),
                3 => self.brewing.is_some_and(|b| b.is_reagent(stack)),
                _ => {
                    let input = self.brewing.is_some_and(|b| b.is_bottle(stack))
                        || self
                            .items
                            .item_tags
                            .get("minecraft:brewing_potion_inputs")
                            .is_some_and(|m| m.contains(&*stack.item));
                    input && self.get(view, slot).is_none()
                }
            };
        }
        if view.kind != Kind::Furnace {
            return true;
        }
        match slot {
            2 => false,
            1 => {
                let fuel = self
                    .items
                    .get(&stack.item)
                    .is_some_and(|i| i.fuel.is_some());
                let current_bucket = self
                    .get(view, 1)
                    .is_some_and(|s| &*s.item == "minecraft:bucket");
                fuel || (&*stack.item == "minecraft:bucket" && !current_bucket)
            }
            _ => true,
        }
    }
    /// `canTakeItemThroughFace` (furnaces and brewing stands restrict it).
    fn can_take(&self, view: &View, slot: usize, stack: &ItemStack, face: Direction) -> bool {
        if let Kind::Composter(level) = view.kind {
            // OutputContainer.canTakeItemThroughFace.
            return level == 8 && face == Direction::Down && &*stack.item == "minecraft:bone_meal";
        }
        if view.kind == Kind::Brewing {
            return slot != 3 || &*stack.item == "minecraft:glass_bottle";
        }
        view.kind != Kind::Furnace
            || face != Direction::Down
            || slot != 1
            || matches!(&*stack.item, "minecraft:water_bucket" | "minecraft:bucket")
    }
    /// `HopperBlockEntity.isFullContainer`.
    fn is_full(&self, view: &View, face: Direction) -> bool {
        view.slots_for_face(Some(face)).into_iter().all(|i| {
            self.get(view, i)
                .is_some_and(|s| s.count >= self.max_stack(s))
        })
    }
    /// `AbstractContainerMenu.getRedstoneSignalFromContainer`.
    fn signal(&self, view: &View) -> i32 {
        if view.kind == Kind::Crafter {
            // CrafterBlockEntity.getRedstoneSignal: filled or disabled slots.
            return match self.world.block_entities.get(&view.parts[0].0) {
                Some(BlockEntity::Crafter(c)) => c.redstone_signal(),
                _ => 0,
            };
        }
        let mut total = 0.0f32;
        for i in 0..view.len() {
            if let Some(s) = self.get(view, i) {
                total += s.count as f32 / self.max_stack(s).min(64) as f32;
            }
        }
        total /= view.len().max(1) as f32;
        // Mth.lerpDiscrete(total, 0, 15)
        (total * 14.0).floor() as i32 + i32::from(total > 0.0)
    }
}

/// The block columns any item entity's box reaches, so that a hopper with
/// no item over it skips the scan of every item. Kept for one block-entity
/// pass, in which items do not move: removing one leaves a superset, still
/// right, and adding one (always pushed at the end, under a new id) or
/// removing the last rebuilds the columns.
#[derive(Default)]
pub(super) struct ItemColumns {
    /// The item count and last item id when last asked, if built.
    seen: Option<(usize, Option<u32>)>,
    cells: horizon_client::fast_hash::FxHashSet<(i32, i32)>,
}
impl ItemColumns {
    /// Whether an item's box may reach into column (x, z).
    fn may_reach(
        &mut self,
        items: &[horizon_client::entities::ItemEntity],
        x: i32,
        z: i32,
    ) -> bool {
        let (len, last) = (items.len(), items.last().map(|i| i.id));
        let stale = self
            .seen
            .is_none_or(|(seen_len, seen_last)| len > seen_len || last != seen_last);
        self.seen = Some((len, last));
        if stale {
            self.cells.clear();
            for item in items {
                let b = item.bounds();
                let (lo, hi) = (b.min_position(), b.max_position());
                for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
                    for cz in lo.z.floor() as i32..=hi.z.floor() as i32 {
                        self.cells.insert((cx, cz));
                    }
                }
            }
        }
        self.cells.contains(&(x, z))
    }
}

fn bone_meal() -> ItemStack {
    ItemStack {
        item: "minecraft:bone_meal".into(),
        count: 1,
        damage: 0,
        extra: None,
    }
}

/// Comparator input from a container block entity (0 when there is none).
pub(super) fn container_signal(
    world: &World,
    behaviours: &Behaviours,
    items: &Items,
    registry: &'static horizon_world::registry::BlockRegistry,
    pos: BlockPos,
) -> i32 {
    let c = Containers {
        world,
        behaviours,
        items,
        registry,
        brewing: None,
        bone_meal: bone_meal(),
    };
    c.view((pos.x, pos.y, pos.z), true)
        .filter(|v| !matches!(v.kind, Kind::Composter(_)))
        .map_or(0, |v| c.signal(&v))
}

impl Game<'_> {
    fn containers(&self) -> Containers<'_> {
        Containers {
            world: &self.world,
            behaviours: self.res.behaviours(),
            items: self.res.items(),
            registry: self.pack.assets.registry,
            brewing: Some(&self.res.stations().brewing),
            bone_meal: bone_meal(),
        }
    }
    fn slot_mut_at(&mut self, view: &View, slot: usize) -> Option<&mut Option<ItemStack>> {
        let (pos, i) = view.locate(slot)?;
        self.world
            .block_entities
            .get_mut(&pos)?
            .slots_mut()
            .get_mut(i)
    }
    /// `BlockEntity.setChanged` for every block entity of a container.
    fn view_changed(&mut self, view: &View) {
        // A composter's containers change its block state instead.
        if let Kind::Composter(_) = view.kind {
            return;
        }
        for &(pos, _) in &view.parts {
            self.container_changed(pos);
        }
    }
    /// `BlockEntity.setChanged` at `pos`: comparators re-read the container.
    pub(super) fn container_changed(&mut self, pos: Pos) {
        if pos.1 == super::minecarts::CART_KEY_Y {
            return;
        }
        // A double chest's other half changes with it.
        let halves: Vec<Pos> = std::iter::once(pos)
            .chain(self.chest_other_half(pos))
            .collect();
        for pos in halves {
            self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
            self.with_level(|b, level, ticks| {
                b.container_changed(level, ticks, BlockPos::new(pos.0, pos.1, pos.2))
            });
        }
    }
    /// `HopperBlockEntity.tryMoveInItem`.
    fn try_move_in_item(
        &mut self,
        from_hopper: Option<Pos>,
        view: &View,
        mut stack: ItemStack,
        slot: usize,
    ) -> Option<ItemStack> {
        let c = self.containers();
        if !c.can_place(view, slot, &stack) {
            return Some(stack);
        }
        let was_empty = c.is_empty(view);
        let max = c.max_stack(&stack);
        let mut success = false;
        let target = self.slot_mut_at(view, slot)?;
        match target {
            None => {
                *target = Some(stack.clone());
                stack.count = 0;
                success = true;
            }
            Some(current)
                // HopperBlockEntity.canMergeItems
                if current.count <= max && current.same_item_same_components(&stack) =>
            {
                let space = max.saturating_sub(current.count);
                let count = stack.count.min(space);
                stack.count -= count;
                current.count += count;
                success = count > 0;
            }
            Some(_) => {}
        }
        if success {
            if was_empty && view.kind == Kind::Hopper {
                let dest = view.parts[0].0;
                let from_ticked = from_hopper.and_then(|p| self.hoppers.get(&p).map(|h| h.ticked));
                let state = self.hoppers.entry(dest).or_default();
                if state.cooldown <= 8 {
                    let skip = i32::from(from_ticked.is_some_and(|t| state.ticked >= t));
                    state.cooldown = 8 - skip;
                }
            }
            self.view_changed(view);
        }
        (stack.count > 0).then_some(stack)
    }
    /// `HopperBlockEntity.addItem(from, container, stack, direction)`; the remainder.
    fn add_item(
        &mut self,
        from_hopper: Option<Pos>,
        view: &View,
        stack: ItemStack,
        face: Option<Direction>,
    ) -> Option<ItemStack> {
        if let Kind::Composter(_) = view.kind {
            let (x, y, z) = view.parts[0].0;
            return self.composter_container_insert([x, y, z], stack, face == Some(Direction::Up));
        }
        let slots = if matches!(view.kind, Kind::Furnace | Kind::Brewing) && face.is_some() {
            view.slots_for_face(face)
        } else {
            (0..view.len()).collect()
        };
        let mut rest = Some(stack);
        for slot in slots {
            let Some(stack) = rest.take() else { break };
            rest = self.try_move_in_item(from_hopper, view, stack, slot);
        }
        rest
    }
    /// Remove one item from a slot (`removeItem(slot, 1)`).
    fn take_one(&mut self, view: &View, slot: usize) -> Option<ItemStack> {
        let target = self.slot_mut_at(view, slot)?;
        let current = target.as_mut()?;
        let one = ItemStack {
            count: 1,
            ..current.clone()
        };
        current.count -= 1;
        if current.count == 0 {
            *target = None;
        }
        Some(one)
    }
    /// Undo `take_one` after a failed move.
    fn put_back(&mut self, view: &View, slot: usize, one: ItemStack) {
        if let Some(target) = self.slot_mut_at(view, slot) {
            match target {
                Some(current) => current.count += one.count,
                None => *target = Some(one),
            }
        }
    }
    /// `HopperBlockEntity.pushItemsTick` of the hopper at `pos`, if its block
    /// is still a hopper (`BlockEntityType.isValid`).
    pub(super) fn tick_hopper(&mut self, pos: Pos) {
        let hopper = self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name
            .as_ref()
            == "minecraft:hopper";
        if !hopper {
            self.hoppers.remove(&pos);
            return;
        }
        let time = self.world.game_time;
        let state = self.hoppers.entry(pos).or_default();
        state.cooldown -= 1;
        state.ticked = time;
        if state.cooldown <= 0 {
            state.cooldown = 0;
            self.hopper_try_move_items(pos);
        }
    }
    /// Drops the cooldowns of hoppers whose block entity is gone.
    pub(super) fn forget_gone_hoppers(&mut self) {
        let entities = &self.world.block_entities;
        self.hoppers.retain(|p, _| {
            matches!(entities.get(p), Some(BlockEntity::Chest(items)) if items.len() == HOPPER_SLOTS)
        });
    }
    /// `HopperBlockEntity.tryMoveItems` with `suckInItems`.
    pub(super) fn hopper_try_move_items(&mut self, pos: Pos) {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let c = self.containers();
        let enabled = c.prop(state, "enabled") != Some("false");
        let Some(view) = c.view(pos, false) else {
            return;
        };
        if self.hoppers.get(&pos).is_some_and(|h| h.cooldown > 0) || !enabled {
            return;
        }
        let mut changed = false;
        if !c.is_empty(&view) {
            changed = self.hopper_eject(pos, &view);
        }
        let c = self.containers();
        let full =
            (0..view.len()).all(|i| c.get(&view, i).is_some_and(|s| s.count == c.max_stack(s)));
        if !full {
            changed |= self.hopper_suck(pos, &view);
        }
        if changed {
            self.hoppers.entry(pos).or_default().cooldown = 8;
            self.container_changed(pos);
        }
    }
    /// `HopperBlockEntity.ejectItems`.
    fn hopper_eject(&mut self, pos: Pos, hopper: &View) -> bool {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let c = self.containers();
        let Some(facing) = c.prop(state, "facing").and_then(dir_from_name) else {
            return false;
        };
        let Some(target) = c.view(offset(pos, facing), false) else {
            return false;
        };
        let face = facing.opposite();
        if c.is_full(&target, face) {
            return false;
        }
        for slot in 0..hopper.len() {
            let Some(one) = self.take_one(hopper, slot) else {
                continue;
            };
            match self.add_item(Some(pos), &target, one, Some(face)) {
                None => {
                    self.view_changed(&target);
                    return true;
                }
                Some(rest) => self.put_back(hopper, slot, rest),
            }
        }
        false
    }
    /// `HopperBlockEntity.suckInItems`: from the container above, or item entities.
    fn hopper_suck(&mut self, pos: Pos, hopper: &View) -> bool {
        let above = (pos.0, pos.1 + 1, pos.2);
        let c = self.containers();
        if let Some(source) = c.view(above, false) {
            if let Kind::Composter(level) = source.kind {
                if level != 8 {
                    return false;
                }
                // tryTakeInItemFromSlot on the OutputContainer: removeItem calls
                // its setChanged, which empties the composter, and so does the
                // second setChanged after the move or, when the hopper has no
                // room, the setItem putting the bone meal back into the detached
                // container (whose bone meal is lost with it).
                let bone_meal = c.bone_meal.clone();
                let at = BlockPos::new(above.0, above.1, above.2);
                self.with_level(|b, level, ticks| b.composter_empty(level, ticks, at));
                let moved = self.add_item(None, hopper, bone_meal, None).is_none();
                self.with_level(|b, level, ticks| b.composter_empty(level, ticks, at));
                self.world.modified.insert((above.0 >> 4, above.2 >> 4));
                return moved;
            }
            let source_hopper = (source.kind == Kind::Hopper).then_some(above);
            for slot in source.slots_for_face(Some(Direction::Down)) {
                let c = self.containers();
                let Some(stack) = c.get(&source, slot) else {
                    continue;
                };
                if !c.can_take(&source, slot, stack, Direction::Down) {
                    continue;
                }
                let Some(one) = self.take_one(&source, slot) else {
                    continue;
                };
                match self.add_item(source_hopper, hopper, one, None) {
                    None => {
                        self.view_changed(&source);
                        return true;
                    }
                    Some(rest) => self.put_back(&source, slot, rest),
                }
            }
            return false;
        }
        let above_state = self.world.block(above.0, above.1, above.2);
        let blocked = self.res.facts().get(above_state).full_collision
            && !self
                .res
                .behaviours()
                .tag("minecraft:does_not_block_hoppers")
                .is_some_and(|t| t.contains(self.res.behaviours().block(above_state)));
        if blocked {
            return false;
        }
        // Hopper.SUCK_AABB: the hopper bowl and the block above.
        let (min, max) = (
            [
                f64::from(pos.0),
                f64::from(pos.1) + 11.0 / 16.0,
                f64::from(pos.2),
            ],
            [
                f64::from(pos.0) + 1.0,
                f64::from(pos.1) + 2.0,
                f64::from(pos.2) + 1.0,
            ],
        );
        if let Some(columns) = self.item_columns.as_mut()
            && !columns.may_reach(&self.dropped, pos.0, pos.2)
        {
            return false;
        }
        let mut i = 0;
        while i < self.dropped.len() {
            let b = self.dropped[i].bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let inside = lo.x < max[0]
                && hi.x > min[0]
                && lo.y < max[1]
                && hi.y > min[1]
                && lo.z < max[2]
                && hi.z > min[2];
            if inside {
                let stack = self.dropped[i].stack.clone();
                match self.add_item(None, hopper, stack, None) {
                    None => {
                        self.dropped.remove(i);
                        return true;
                    }
                    Some(rest) => self.dropped[i].stack = rest,
                }
            }
            i += 1;
        }
        false
    }

    // ---- dispensers and droppers ----

    /// `DispenserBlock.dispenseFrom` / `DropperBlock.dispenseFrom`.
    pub(super) fn dispense_from(&mut self, pos: BlockPos, state: BlockStateId) -> Result<()> {
        let key = (pos.x, pos.y, pos.z);
        let name = self.res.facts().get(state).name.clone();
        if &*name == "minecraft:crafter" {
            return self.crafter_dispense(pos, state);
        }
        let dropper = &*name == "minecraft:dropper";
        if !dropper && &*name != "minecraft:dispenser" {
            return Ok(());
        }
        let Some(view) = self.containers().view(key, false) else {
            return Ok(());
        };
        // DispenserBlockEntity.getRandomSlot
        let mut slot = None;
        let mut odds = 1;
        for i in 0..view.len() {
            if self.containers().get(&view, i).is_some() {
                if self.world.random.next_int_bound(odds).unwrap_or(1) == 0 {
                    slot = Some(i);
                }
                odds += 1;
            }
        }
        let Some(slot) = slot else {
            self.dispenser_event(1001, key, 0);
            if !dropper {
                self.post_block_game_event(
                    "minecraft:block_activate",
                    [pos.x, pos.y, pos.z],
                    None,
                    Some(state),
                );
            }
            return Ok(());
        };
        let Some(facing) = self
            .containers()
            .prop(state, "facing")
            .and_then(dir_from_name)
        else {
            return Ok(());
        };
        let Some(stack) = self.containers().get(&view, slot).cloned() else {
            return Ok(());
        };
        let target = offset(key, facing);
        let remaining = if dropper {
            match self.containers().view(target, false) {
                None => self.dispense_default(key, facing, stack),
                Some(into) => {
                    let one = ItemStack {
                        count: 1,
                        ..stack.clone()
                    };
                    if self
                        .add_item(None, &into, one, Some(facing.opposite()))
                        .is_none()
                    {
                        self.view_changed(&into);
                        Some(ItemStack {
                            count: stack.count - 1,
                            ..stack
                        })
                        .filter(|s| s.count > 0)
                    } else {
                        Some(stack)
                    }
                }
            }
        } else {
            self.dispense_item(key, facing, stack)?
        };
        if let Some(entry) = self.slot_mut_at(&view, slot) {
            *entry = remaining;
        }
        self.container_changed(key);
        Ok(())
    }
}

// ---- hopper, dispenser and dropper screens ----

/// Which machine screen the open container uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Hopper,
    Dispenser,
    Dropper,
}

impl Game<'_> {
    fn machine_screen(&self) -> Option<Screen> {
        let key = self.container?;
        // A server's menu shows by its type, whatever block (or minecart) holds it.
        if let Some(menu) = self.remote_menu_type() {
            return match menu {
                "minecraft:hopper" => Some(Screen::Hopper),
                "minecraft:generic_3x3" => Some(Screen::Dispenser),
                _ => None,
            };
        }
        if key.1 == super::minecarts::CART_KEY_Y {
            return (self.open_cart_kind() == Some(super::minecarts::CartKind::Hopper))
                .then_some(Screen::Hopper);
        }
        let state = self.world.block(key.0, key.1, key.2);
        match &*self.res.facts().get(state).name {
            "minecraft:hopper" => Some(Screen::Hopper),
            "minecraft:dispenser" => Some(Screen::Dispenser),
            "minecraft:dropper" => Some(Screen::Dropper),
            _ => None,
        }
    }
    /// `imageHeight` of the open screen (HopperScreen is 133 tall).
    pub(super) fn screen_height(&self) -> f32 {
        if self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == super::stations::Kind::Beacon)
        {
            return super::beacon::HEIGHT;
        }
        if let Some(rows) = self.chest_rows() {
            // ContainerScreen: 114 + 18 per row.
            return 114.0 + rows as f32 * 18.0;
        }
        if self.container_block_is_shulker() {
            return 167.0;
        }
        if self.machine_screen() == Some(Screen::Hopper) {
            133.0
        } else {
            166.0
        }
    }
    /// HopperMenu / DispenserMenu slot layouts; None for other screens.
    pub(super) fn machine_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        let screen = self.machine_screen()?;
        let inventory_y = if screen == Screen::Hopper { 51.0 } else { 84.0 };
        Some(match i {
            0..9 => (8.0 + i as f32 * 18.0, inventory_y + 58.0),
            9..36 => (
                8.0 + ((i - 9) % 9) as f32 * 18.0,
                inventory_y + ((i - 9) / 9) as f32 * 18.0,
            ),
            300..500 if screen == Screen::Hopper => (44.0 + (i - 300) as f32 * 18.0, 20.0),
            300..500 => (
                62.0 + ((i - 300) % 3) as f32 * 18.0,
                17.0 + ((i - 300) / 3) as f32 * 18.0,
            ),
            _ => return None,
        })
    }
    /// HopperScreen / DispenserScreen background and labels; false for other containers.
    pub(super) fn draw_machine_screen(
        &self,
        batch: &mut horizon_client::hud::HudBatch,
        ox: f32,
        oy: f32,
    ) -> bool {
        let Some(screen) = self.machine_screen() else {
            return false;
        };
        let atlas = &self.gui.atlas;
        let key_is_cart = self
            .container
            .is_some_and(|k| k.1 == super::minecarts::CART_KEY_Y);
        let (texture, height, key, fallback) = match screen {
            Screen::Hopper if key_is_cart => (
                "gui/container/hopper",
                133.0,
                "entity.minecraft.hopper_minecart",
                "Minecart with Hopper",
            ),
            Screen::Hopper => (
                "gui/container/hopper",
                133.0,
                "container.hopper",
                "Item Hopper",
            ),
            Screen::Dispenser => (
                "gui/container/dispenser",
                166.0,
                "container.dispenser",
                "Dispenser",
            ),
            Screen::Dropper => (
                "gui/container/dispenser",
                166.0,
                "container.dropper",
                "Dropper",
            ),
        };
        if let Some(mut r) = atlas.sprites.get(texture).copied() {
            let dv = (r.v1 - r.v0) / r.h as f32;
            let du = (r.u1 - r.u0) / r.w as f32;
            r.u1 = r.u0 + 176.0 * du;
            r.v1 = r.v0 + height * dv;
            batch.rect_uv(ox, oy, 176.0, height, r, u32::MAX);
        }
        let grey = horizon_client::hud::rgba(64, 64, 64, 255);
        let title = &self.remote_menu_title().unwrap_or_else(|| {
            horizon_client::text::Text::literal(self.container_title(self.translate(key, fallback)))
                .resolve_legacy()
        });
        let title_x = if screen == Screen::Hopper {
            8.0
        } else {
            (176.0 - atlas.font.width_text(title)) / 2.0
        };
        batch.text_styled(atlas, title, ox + title_x, oy + 6.0, grey, false);
        let inventory = self.translate("container.inventory", "Inventory");
        batch.text(atlas, &inventory, ox + 8.0, oy + height - 94.0, grey, false);
        true
    }
}

#[cfg(test)]
mod item_column_tests {
    use super::ItemColumns;
    use horizon_client::{entities::ItemEntity, items::ItemStack};
    use horizon_core::geometry::Vec3;

    fn item(id: u32, x: f64, y: f64, z: f64) -> ItemEntity {
        let at = Vec3::new(x, y, z);
        ItemEntity {
            health: 5,
            fire_ticks: 0,
            id,
            stack: ItemStack::new("minecraft:stone", 1),
            thrower: None,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: true,
            age: 0,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
        }
    }
    /// `hopper_suck`'s test: some item box meets the hopper's SUCK_AABB.
    fn scan(items: &[ItemEntity], (x, y, z): (i32, i32, i32)) -> bool {
        let (min, max) = (
            Vec3::new(f64::from(x), f64::from(y) + 11.0 / 16.0, f64::from(z)),
            Vec3::new(f64::from(x) + 1.0, f64::from(y) + 2.0, f64::from(z) + 1.0),
        );
        items.iter().any(|i| {
            let b = i.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            lo.x < max.x
                && hi.x > min.x
                && lo.y < max.y
                && hi.y > min.y
                && lo.z < max.z
                && hi.z > min.z
        })
    }

    /// One tick of empty hoppers looking for items: each scans every item
    /// (before) or asks the columns first (after, build included).
    #[test]
    #[ignore]
    fn hopper_suck_cost() {
        for (hoppers, count) in [(500i32, 200u32), (2000, 500), (2000, 2000)] {
            let mut seed = 99u64;
            let mut next = |range: f64| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (seed >> 11) as f64 / (1u64 << 53) as f64 * range
            };
            let items: Vec<ItemEntity> = (0..count)
                .map(|id| item(id, next(160.0), 64.0, next(160.0)))
                .collect();
            let at: Vec<(i32, i32, i32)> = (0..hoppers)
                .map(|_| (next(160.0) as i32, 63, next(160.0) as i32))
                .collect();
            let t = std::time::Instant::now();
            let a: Vec<bool> = at.iter().map(|&h| scan(&items, h)).collect();
            let before = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let mut columns = ItemColumns::default();
            let b: Vec<bool> = at
                .iter()
                .map(|&h| columns.may_reach(&items, h.0, h.2) && scan(&items, h))
                .collect();
            let after = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(a, b);
            println!("{hoppers} hoppers, {count} items: {before:.3} ms -> {after:.3} ms");
        }
    }

    #[test]
    fn no_hopper_with_an_item_over_it_is_skipped() {
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut items: Vec<ItemEntity> = Vec::new();
        let mut columns = ItemColumns::default();
        let mut next_id = 0;
        let mut skipped = 0;
        for round in 0..3000 {
            // Items come and go as hoppers take them and crafters drop them.
            match round % 5 {
                0 | 1 => {
                    next_id += 1;
                    // On edges too: a quarter of them sit on a block line.
                    let snap = |v: f64, edge: bool| if edge { v.floor() + 0.125 } else { v };
                    let edge = round % 4 == 0;
                    items.push(item(
                        next_id,
                        snap(next(24.0) - 12.0, edge),
                        64.0,
                        snap(next(24.0) - 12.0, edge),
                    ));
                }
                2 if !items.is_empty() => {
                    let i = next(items.len() as f64) as usize;
                    items.remove(i);
                }
                _ => {}
            }
            let hopper = (next(26.0) as i32 - 13, 63, next(26.0) as i32 - 13);
            let reach = columns.may_reach(&items, hopper.0, hopper.2);
            let found = scan(&items, hopper);
            assert!(
                reach || !found,
                "round {round}: skipped a hopper with an item over it"
            );
            skipped += usize::from(!reach);
        }
        assert!(skipped > 1000, "skipped only {skipped}");
    }
}
