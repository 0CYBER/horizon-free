//! Item-moving blocks: `HopperBlockEntity` (push, pull, item entities,
//! transfer cooldowns), `DispenserBlock` / `DropperBlock.dispenseFrom` with
//! the vanilla dispense behaviours Horizon has entities for, and container
//! comparator signals (`AbstractContainerMenu.getRedstoneSignalFromContainer`).
mod loot_access;
use super::{Game, composter::is_composter, spill, storage::is_shulker_box};
use horizon_core::nbt::Tag;
use mod_api::__private::game::*;

pub(in crate::game) mod cauldron_contents;
#[cfg(test)]
mod contents_changed_tests;
mod crafter;
pub(in crate::game) mod native_energy;
pub(in crate::game) mod native_fluids;
pub(in crate::game) mod native_items;
mod screen;
#[cfg(test)]
mod tank_reopen_tests;
use horizon_client::{
    Result,
    block_contact::sweep_movement_segments,
    containers::{BlockEntity, HOPPER_SLOTS, ModContainer},
    fast_hash::FxHashMap,
    items::{ItemStack, Items},
    mobs::Mob,
    save,
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3, sweep_intersected_blocks},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    entity_move::MovementRecord,
    level::{Behaviours, LevelAccess},
    registry::BlockStateId,
};
use mod_api::{
    self, BlockSide, Capability, ENTITY_AUTOMATION, EntityRef, ItemResource, ResourceHandler,
};
use std::{borrow::Cow, ops::ControlFlow, sync::Arc};

type Pos = (i32, i32, i32);

/// What item automation reaches among the entities at its target (`ENTITY_AUTOMATION`).
enum AutomationEntity {
    /// A container entity the game answers for, by id: reached as a vanilla container.
    Container(u32),
    /// An entity a mod's provider answers for, with its handler.
    Part(EntityRef, Capability<dyn ResourceHandler<ItemResource>>),
}

/// A block or entity destination, reached through its slots or a mod's handler.
enum ContainerTarget<'a> {
    View(&'a View),
    Part(Capability<dyn ResourceHandler<ItemResource>>),
}

/// `HopperBlockEntity` transient state (`TransferCooldown`, `tickedGameTime`).
#[derive(Clone, Copy, Debug)]
pub(super) struct HopperState {
    pub(super) cooldown: i32,
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

impl HopperState {
    /// A transferred holder keeps its cooldown and starts with a fresh ticker timestamp.
    pub(super) fn with_cooldown(cooldown: i32) -> Self {
        Self {
            cooldown,
            ticked: 0,
        }
    }

    /// `HopperBlockEntity.loadAdditional`: only the cooldown is saved; the
    /// last tick time starts at zero for the new block entity.
    pub(super) fn load(tag: &horizon_core::nbt::Tag) -> Self {
        let canonical = horizon_client::block_entity_nbt::canonical(tag);
        let cooldown = match canonical.as_ref().and_then(|t| t.get("TransferCooldown")) {
            Some(horizon_core::nbt::Tag::Int(value)) => *value,
            _ => -1,
        };
        Self::with_cooldown(cooldown)
    }
}

impl Game<'_> {
    pub(super) fn hopper_cooldown(&self, pos: Pos) -> i32 {
        self.hoppers.get(&pos).map_or_else(
            || {
                self.world
                    .chunks
                    .get(&(pos.0 >> 4, pos.2 >> 4))
                    .and_then(|c| c.saved.as_deref())
                    .and_then(|s| s.block_entity_sources.iter().find(|(p, _)| *p == pos))
                    .map_or(-1, |(_, tag)| HopperState::load(tag).cooldown)
            },
            |s| s.cooldown,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Generic,
    /// The saved decorated-pot slot: hopper insertion and the comparator signal.
    Pot,
    /// `AbstractFurnaceBlockEntity` (a `WorldlyContainer`).
    Furnace,
    Hopper,
    /// `CrafterBlockEntity`: disabled slots and the smallest-stack-first rule.
    Crafter,
    /// `BrewingStandBlockEntity` (a `WorldlyContainer`).
    Brewing,
    /// `ChiseledBookShelfBlockEntity`: books only, one to a slot.
    Bookshelf,
    /// `JukeboxBlockEntity`'s single slot: a playable disc only.
    Jukebox,
    /// A mod block's block entity (`ModContainer`).
    Mod,
    /// `ShulkerBoxBlockEntity`: a shulker box doesn't go through a face into one.
    Shulker,
    /// `ComposterBlock.getContainer` at this level (a `WorldlyContainerHolder`
    /// without a block entity): input below 7, empty at 7, bone meal at 8.
    Composter(i32),
}

/// A container: one block entity, or a double chest (`CompoundContainer`).
#[derive(Clone, Debug)]
struct View {
    parts: Vec<(Pos, usize)>,
    kind: Kind,
    /// A mod block's container rules (`Kind::Mod`).
    container: Option<Arc<ModContainer>>,
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
    /// `WorldlyContainer.getSlotsForFace` or every slot, zero-allocation iterator.
    pub(super) fn slots_for_face_iter(&self, face: Option<Direction>) -> Slots<'_> {
        match (self.kind, face) {
            (Kind::Furnace, Some(Direction::Down)) => Slots::Slice(&[2, 1]),
            (Kind::Furnace, Some(Direction::Up)) => Slots::Slice(&[0]),
            (Kind::Furnace, Some(_)) => Slots::Slice(&[1]),
            (Kind::Brewing, Some(Direction::Up)) => Slots::Slice(&[3]),
            (Kind::Brewing, Some(Direction::Down)) => Slots::Slice(&[0, 1, 2, 3]),
            (Kind::Brewing, Some(_)) => Slots::Slice(&[0, 1, 2, 4]),
            (Kind::Composter(8), Some(Direction::Down)) => Slots::Slice(&[0]),
            (Kind::Composter(level), Some(Direction::Up)) if level < 7 => Slots::Slice(&[0]),
            (Kind::Composter(_), _) => Slots::Slice(&[]),
            (Kind::Mod, Some(face)) => self
                .container
                .as_ref()
                .map_or(Slots::Slice(&[]), |c| Slots::Mod(&c.faces[face as usize])),
            _ => Slots::Range(0..self.len()),
        }
    }
    /// `Container.getMaxStackSize`: what one slot holds of any item, 1 for a chiseled
    /// bookshelf's books and a jukebox's disc, else vanilla's default.
    fn slot_limit(&self) -> u32 {
        match self.kind {
            Kind::Bookshelf | Kind::Jukebox | Kind::Composter(_) => 1,
            _ => 99,
        }
    }
    pub(super) fn slots_to_add_iter(&self, face: Option<Direction>) -> Slots<'_> {
        if matches!(self.kind, Kind::Furnace | Kind::Brewing | Kind::Mod) && face.is_some() {
            self.slots_for_face_iter(face)
        } else {
            Slots::Range(0..self.len())
        }
    }
    #[cfg(test)]
    fn slots_to_add(&self, face: Option<Direction>) -> Vec<usize> {
        self.slots_to_add_iter(face).collect()
    }
}

pub(super) enum Slots<'a> {
    Range(std::ops::Range<usize>),
    Slice(&'static [usize]),
    Mod(&'a [usize]),
}

impl Iterator for Slots<'_> {
    type Item = usize;
    #[inline]
    fn next(&mut self) -> Option<usize> {
        match self {
            Slots::Range(r) => r.next(),
            Slots::Slice(s) => {
                let (&first, rest) = s.split_first()?;
                *s = rest;
                Some(first)
            }
            Slots::Mod(s) => {
                let (&first, rest) = s.split_first()?;
                *s = rest;
                Some(first)
            }
        }
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match self {
            Slots::Range(r) => r.len(),
            Slots::Slice(s) => s.len(),
            Slots::Mod(s) => s.len(),
        };
        (len, Some(len))
    }
}
impl ExactSizeIterator for Slots<'_> {}

pub(super) fn dir_from_name(name: &str) -> Option<Direction> {
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
/// State-level container queries shared by the game and comparator signals.
pub(super) struct Containers<'a> {
    world: &'a World,
    behaviours: &'a Behaviours,
    items: &'a Items,
    registry: &'static horizon_world::registry::BlockRegistry,
    /// Brewing recipes, for what a brewing stand accepts (not needed for signals).
    brewing: Option<&'a horizon_client::brewing::Brewing>,
    /// The level's mobs, for the cat half of `ChestBlock.isChestBlockedAt`.
    mobs: &'a [Mob],
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
    /// `ItemStack.getMaxStackSize`: its component (a removed one being
    /// vanilla's default of 1), else the item's.
    pub(super) fn max_stack(&self, stack: &ItemStack) -> u32 {
        stack.max_stack_size(self.items).max(1)
    }
    /// `Container.getMaxStackSize(stack)` of a container: the container's own
    /// limit, at most the item's. `ComposterBlock.InputContainer` (levels 0 to
    /// 6) and `OutputContainer` (level 8, `READY`), `JukeboxBlockEntity` and
    /// `ChiseledBookShelfBlockEntity` override `getMaxStackSize` to 1; every
    /// other container keeps the `Container` default of 99. A composter's
    /// level 7 is `EmptyContainer` (99), which takes nothing through a face.
    fn max_stack_in(&self, view: &View, stack: &ItemStack) -> u32 {
        let container = match view.kind {
            Kind::Composter(0..=6) | Kind::Composter(8) | Kind::Jukebox | Kind::Bookshelf => 1,
            _ => 99,
        };
        container.min(self.max_stack(stack))
    }
    /// `HopperBlockEntity.getBlockContainer` (double chests combined, right half
    /// first). `blocked_chests`: `ChestBlock.getContainer(.., false)` returns
    /// nothing for a blocked chest (`isChestBlockedAt`: a redstone conductor
    /// above it or a cat sitting on it).
    fn view(&self, pos: Pos, blocked_chests: bool) -> Option<View> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let name = self.name(state);
        if is_composter(name) {
            let level = self.prop(state, "level").and_then(|l| l.parse().ok())?;
            return Some(View {
                parts: vec![(pos, usize::from(level != 7))],
                kind: Kind::Composter(level),
                container: None,
            });
        }
        let entity = self.world.block_entities.get(&pos)?;
        // A mod's block entity is a container when its block says so. The one
        // ticking now is out of the world (an empty id marks its place): what
        // went into its place would be lost when it comes back.
        if let BlockEntity::Mod(m) = entity {
            if m.id().is_empty() {
                return None;
            }
            let container = Arc::clone(self.world.block_entities.mod_container(state.0)?);
            return Some(View {
                parts: vec![(pos, container.slots)],
                kind: Kind::Mod,
                container: Some(container),
            });
        }
        // Non-container block entities (signs, banners, beacons, skulls, etc.,
        // or beehives/campfires/brushables) are not `Container`s
        // (HopperBlockEntity.getContainerAt), so a dropper drops in front of
        // them and a hopper leaves them alone.
        let size = entity.slots().len();
        if size == 0 {
            return None;
        }
        let kind = match (entity, name) {
            (BlockEntity::Furnace(_), _) => Kind::Furnace,
            (BlockEntity::Crafter(_), _) => Kind::Crafter,
            (BlockEntity::Brewing(_), _) => Kind::Brewing,
            (BlockEntity::ChiseledBookshelf(_), _) => Kind::Bookshelf,
            (BlockEntity::Jukebox(_), _) => Kind::Jukebox,
            (_, "minecraft:hopper") => Kind::Hopper,
            (_, n) if is_shulker_box(n) => Kind::Shulker,
            _ => Kind::Generic,
        };
        if horizon_client::containers::is_chest_block(name) {
            let is_blocked = |p: Pos| {
                let above = self.world.block(p.0, p.1 + 1, p.2);
                self.behaviours.facts(above).flags
                    & horizon_world::level::data::state_flags::CONDUCTOR
                    != 0
                    || cat_sitting_above(self.mobs, p)
            };
            if blocked_chests && is_blocked(pos) {
                return None;
            }
            let half = |s: BlockStateId| {
                Some(horizon_client::containers::ChestHalf {
                    block: self.name(s),
                    kind: self.prop(s, "type")?,
                    facing: self.prop(s, "facing")?,
                })
            };
            if let Some(own) = half(state)
                && let Some((dx, dz)) = own.connected_step()
            {
                let other = (pos.0 + dx, pos.1, pos.2 + dz);
                let other_state = self.world.block(other.0, other.1, other.2);
                let paired = half(other_state).is_some_and(|o| own.pairs_with(&o))
                    && self
                        .world
                        .block_entities
                        .get(&other)
                        .is_some_and(|e| e.slots().len() == 27);
                if paired {
                    if blocked_chests && is_blocked(other) {
                        return None;
                    }
                    let parts = if own.kind == "right" {
                        vec![(pos, size), (other, 27)]
                    } else {
                        vec![(other, 27), (pos, size)]
                    };
                    return Some(View {
                        parts,
                        kind,
                        container: None,
                    });
                }
            }
        }
        Some(View {
            parts: vec![(pos, size)],
            kind,
            container: None,
        })
    }
    /// The saved decorated-pot slot as a container: `DecoratedPotBlockEntity`
    /// is a `ContainerSingleItem`, but Horizon keeps pots out of generic
    /// container discovery (`world.block_entities`), so this is built on demand.
    /// A pot with pending loot is left to its existing unpacking path.
    fn saved_pot_view(&self, pos: Pos) -> Option<View> {
        if self.name(self.world.block(pos.0, pos.1, pos.2)) != "minecraft:decorated_pot"
            || spill::saved_entity(self.world, [pos.0, pos.1, pos.2])
                .is_some_and(|tag| matches!(tag.get("LootTable"), Some(Tag::String(_))))
        {
            return None;
        }
        Some(View {
            parts: vec![(pos, 1)],
            kind: Kind::Pot,
            container: None,
        })
    }
    /// `HopperBlockEntity.getBlockContainer` at its insertion target only.
    /// Pots keep their authoritative slot in the chunk's saved tag.
    fn hopper_insertion_view(&self, pos: Pos) -> Option<View> {
        self.view(pos, false).or_else(|| self.saved_pot_view(pos))
    }
    /// The `ContainerSingleItem` slot for hopper insertion: only a saved pot
    /// needs an owned decode. Existing modeled containers stay borrowed.
    fn insertion_item(&self, view: &View, slot: usize) -> Option<Cow<'_, ItemStack>> {
        if view.kind == Kind::Pot {
            let (pos, _) = view.locate(slot)?;
            let tag = spill::saved_entity(self.world, [pos.0, pos.1, pos.2])?;
            return tag
                .get("item")
                .and_then(save::item_from_nbt)
                .filter(|stack| stack.count > 0)
                .map(Cow::Owned);
        }
        self.get(view, slot).map(Cow::Borrowed)
    }
    fn get(&self, view: &View, slot: usize) -> Option<&ItemStack> {
        if let Kind::Composter(level) = view.kind {
            // OutputContainer holds one bone meal; InputContainer composts at once.
            return (level == 8 && slot == 0).then_some(bone_meal_stack());
        }
        let (pos, i) = view.locate(slot)?;
        self.world
            .block_entities
            .get(&pos)?
            .slots()
            .get(i)?
            .as_ref()
            // A mod can leave an empty stack in its own slots.
            .filter(|s| s.count > 0)
    }
    fn is_empty(&self, view: &View) -> bool {
        (0..view.len()).all(|i| self.get(view, i).is_none())
    }
    /// `Container.canPlaceItem` (+ `canPlaceItemThroughFace` for furnaces) with slot `slot`
    /// empty: the item rule of the slot, whatever it holds now.
    fn accepts(&self, view: &View, slot: usize, stack: &ItemStack) -> bool {
        if let Some(c) = &view.container {
            return c.inputs.contains(&slot);
        }
        match view.kind {
            // ComposterBlock.InputContainer.canPlaceItemThroughFace; output never accepts.
            Kind::Composter(level) => {
                level < 7
                    && slot == 0
                    && self
                        .items
                        .get(&stack.item)
                        .is_some_and(|i| i.compostable.is_some())
            }
            // ShulkerBoxBlockEntity.canPlaceItemThroughFace
            Kind::Shulker => !is_shulker_box(&stack.item),
            Kind::Crafter => match self.world.block_entities.get(&view.parts[0].0) {
                Some(BlockEntity::Crafter(c)) => slot < 9 && !c.disabled[slot],
                _ => false,
            },
            // JukeboxBlockEntity.canPlaceItem: a playable disc.
            Kind::Jukebox => self
                .items
                .get(&stack.item)
                .is_some_and(|i| i.jukebox_song.is_some()),
            // ListBackedContainer.canPlaceItem: `acceptsItemType` (the books).
            Kind::Bookshelf => self
                .items
                .item_tags
                .get("minecraft:bookshelf_books")
                .is_some_and(|m| m.contains(&*stack.item)),
            // BrewingStandBlockEntity.canPlaceItem
            Kind::Brewing => match slot {
                4 => horizon_client::brewing::brewing_fuel(self.items, stack).is_some(),
                3 => self.brewing.is_some_and(|b| b.is_reagent(stack)),
                _ => {
                    self.brewing.is_some_and(|b| b.is_bottle(stack))
                        || self
                            .items
                            .item_tags
                            .get("minecraft:brewing_potion_inputs")
                            .is_some_and(|m| m.contains(&*stack.item))
                }
            },
            Kind::Furnace => match slot {
                2 => false,
                1 => {
                    &*stack.item == "minecraft:bucket"
                        || self
                            .items
                            .get(&stack.item)
                            .is_some_and(|i| i.fuel.is_some())
                }
                _ => true,
            },
            _ => true,
        }
    }
    /// `Container.canPlaceItem` (+ `canPlaceItemThroughFace` for furnaces): `accepts`, and what
    /// the slot holds now. The slots of one item (a jukebox's, a chiseled bookshelf's, a brewing
    /// stand's bottles) take one only when empty, a furnace's fuel slot no bucket onto a
    /// bucket, and a crafter fills its smallest stack first.
    fn can_place(&self, view: &View, slot: usize, stack: &ItemStack) -> bool {
        if !self.accepts(view, slot, stack) {
            return false;
        }
        let held = self.get(view, slot);
        let is_bucket = |s: &ItemStack| &*s.item == "minecraft:bucket";
        match view.kind {
            _ if view.container.is_some() => true,
            Kind::Jukebox | Kind::Bookshelf => held.is_none(),
            Kind::Brewing => slot >= 3 || held.is_none(),
            Kind::Furnace => slot != 1 || !is_bucket(stack) || !held.is_some_and(is_bucket),
            Kind::Crafter => match self.world.block_entities.get(&view.parts[0].0) {
                Some(BlockEntity::Crafter(c)) => {
                    // The slot's own stack sets the limit (`slotStack.getMaxStackSize`).
                    let max = c.items.get(slot).and_then(Option::as_ref).unwrap_or(stack);
                    c.can_place(slot, stack, self.max_stack(max))
                }
                _ => false,
            },
            _ => true,
        }
    }
    /// How many of `stack` slot `slot` holds when empty, whatever it holds now: none where
    /// `accepts` refuses it, else the slot's limit (`Container.getMaxStackSize(stack)`: the
    /// container's and the item's, the lower).
    fn capacity(&self, view: &View, slot: usize, stack: &ItemStack) -> u32 {
        if !self.accepts(view, slot, stack) {
            return 0;
        }
        let capacity = view.slot_limit().min(self.max_stack(stack));
        view.locate(slot)
            .and_then(|(pos, _)| self.world.container_stack_limit(pos))
            .map_or(capacity, |limit| capacity.min(limit))
    }
    /// How many of `stack` slot `slot` takes now: none where `can_place` refuses it, else
    /// `capacity` less what it holds.
    fn fits(&self, view: &View, slot: usize, stack: &ItemStack) -> u32 {
        if !self.can_place(view, slot, stack) {
            return 0;
        }
        room(
            self.capacity(view, slot, stack),
            self.get(view, slot),
            stack,
        )
    }
    /// `canTakeItemThroughFace` (furnaces and brewing stands restrict it).
    /// `Container.canTakeItem` for a hopper's slot of `source`: the vanilla
    /// shelf rule needs to know where the item is going.
    fn can_take_into(
        &self,
        view: &View,
        slot: usize,
        stack: &ItemStack,
        face: Direction,
        target: &View,
    ) -> bool {
        if view.kind == Kind::Bookshelf {
            // `ChiseledBookShelfBlockEntity.canTakeItem`: the target must have
            // a free slot or a matching stack with room (`hasAnyMatching`).
            return (0..target.len()).any(|i| match self.get(target, i) {
                None => true,
                Some(other) => {
                    other.same_item_same_components(stack)
                        && other.count + stack.count <= self.max_stack(other)
                }
            });
        }
        self.can_take(view, slot, stack, face)
    }
    fn can_take(&self, view: &View, slot: usize, stack: &ItemStack, face: Direction) -> bool {
        if let Kind::Composter(level) = view.kind {
            // OutputContainer.canTakeItemThroughFace.
            return level == 8 && face == Direction::Down && &*stack.item == "minecraft:bone_meal";
        }
        if view.kind == Kind::Brewing {
            return slot != 3 || &*stack.item == "minecraft:glass_bottle";
        }
        if let Some(c) = &view.container {
            return c.outputs.contains(&slot);
        }
        view.kind != Kind::Furnace
            || face != Direction::Down
            || slot != 1
            || matches!(&*stack.item, "minecraft:water_bucket" | "minecraft:bucket")
    }
    /// `HopperBlockEntity.isFullContainer`.
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
            // `insertion_item` decodes a saved pot's slot; every other kind is `get`.
            if let Some(s) = self.insertion_item(view, i) {
                total += s.count as f32 / self.max_stack(&s) as f32;
            }
        }
        total /= view.len().max(1) as f32;
        // Mth.lerpDiscrete(total, 0, 15). A mod can write a stack over its own
        // limit, so the ratio itself is clamped, as vanilla's invariant does.
        (total.min(1.0) * 14.0).floor() as i32 + i32::from(total > 0.0)
    }
}

/// How many of `stack` go into a slot holding `current`, up to `limit`: into an empty slot, or
/// onto the same item and components (`HopperBlockEntity.canMergeItems`).
fn room(limit: u32, current: Option<&ItemStack>, stack: &ItemStack) -> u32 {
    match current {
        None => stack.count.min(limit),
        Some(current) if current.same_item_same_components(stack) => {
            stack.count.min(limit.saturating_sub(current.count))
        }
        Some(_) => 0,
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
        if items.is_empty() {
            return false;
        }
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
    /// Whether an item's box may reach into any column of x0..=x1, z0..=z1.
    pub(super) fn may_reach_area(
        &mut self,
        items: &[horizon_client::entities::ItemEntity],
        (x0, x1): (i32, i32),
        (z0, z1): (i32, i32),
    ) -> bool {
        (x0..=x1).any(|x| (z0..=z1).any(|z| self.may_reach(items, x, z)))
    }
}

/// [`ItemColumns`] for a pass in which items can move (a command, an
/// explosion's push), kept with the item list's
/// [`ListStamp`](horizon_client::join_seq::ListStamp): after any write the
/// first question scans as before (`true`), and the second with nothing
/// written in between builds the columns. A pass that writes the items
/// between questions costs no more than the scans did.
#[derive(Default)]
pub(super) struct StampedItemColumns {
    stamp: Option<horizon_client::join_seq::ListStamp>,
    columns: ItemColumns,
}
impl StampedItemColumns {
    /// [`ItemColumns::may_reach_area`] for the items as they are now.
    pub(super) fn may_reach_area(
        &mut self,
        items: &horizon_client::join_seq::JoinList<horizon_client::entities::ItemEntity>,
        xs: (i32, i32),
        zs: (i32, i32),
    ) -> bool {
        if self.stamp != Some(items.stamp()) {
            self.stamp = Some(items.stamp());
            self.columns = ItemColumns::default();
            return true;
        }
        self.columns.may_reach_area(items, xs, zs)
    }
}

fn bone_meal_stack() -> &'static ItemStack {
    static STACK: std::sync::LazyLock<ItemStack> = std::sync::LazyLock::new(|| ItemStack {
        item: "minecraft:bone_meal".into(),
        count: 1,
        damage: 0,
        extra: None,
    });
    &STACK
}

/// `ChestBlock.isCatSittingOnChest`: a sitting cat whose box meets the block
/// above `key` (the box of the block above, not the chest's own).
pub(super) fn cat_sitting_above(mobs: &[Mob], key: Pos) -> bool {
    let area = Aabb::new(
        Vec3::new(f64::from(key.0), f64::from(key.1 + 1), f64::from(key.2)),
        Vec3::new(
            f64::from(key.0 + 1),
            f64::from(key.1 + 2),
            f64::from(key.2 + 1),
        ),
    );
    mobs.iter().any(|m| {
        m.species() == horizon_client::mobs::Species::Cat
            && m.alive()
            && horizon_client::mobs::tame::cat_sitting(m)
            && m.bounds().intersects(area)
    })
}

/// Comparator input from a container block entity (0 when there is none):
/// `ChestBlock.getContainer(.., false)`, whose blocked check sees the level's
/// cats (`isCatSittingOnChest`).
pub(super) fn container_signal(level: &mut super::GameLevel<'_>, pos: BlockPos) -> i32 {
    let c = Containers {
        world: &*level.world,
        behaviours: level.behaviours,
        items: level.items,
        registry: level.registry,
        brewing: None,
        mobs: level.entities().mobs,
    };
    let view = c
        .view((pos.x, pos.y, pos.z), true)
        .filter(|v| !matches!(v.kind, Kind::Composter(_)))
        .or_else(|| c.saved_pot_view((pos.x, pos.y, pos.z)));
    if let Some(view) = &view {
        for &(key, _) in &view.parts {
            level.unpack_loot(super::container_loot::LootTarget::Block(key), None);
        }
    }
    let c = Containers {
        world: &*level.world,
        behaviours: level.behaviours,
        items: level.items,
        registry: level.registry,
        brewing: None,
        mobs: level.entities().mobs,
    };
    view.map_or(0, |v| c.signal(&v))
}

/// `ChiseledBookShelfBlock.getAnalogOutputSignal` and
/// `ShelfBlock.getAnalogOutputSignal`, from the block entity Horizon keeps as
/// it was loaded; `None` for any other block. `direction` points from the
/// block to the comparator.
pub(super) fn shelf_signal(
    world: &World,
    registry: &'static horizon_world::registry::BlockRegistry,
    state: BlockStateId,
    pos: BlockPos,
    direction: Direction,
) -> Option<i32> {
    let info = registry.state(state)?;
    let chiseled = info.name == "minecraft:chiseled_bookshelf";
    if !chiseled && !info.name.ends_with("_shelf") {
        return None;
    }
    if chiseled {
        // The last slot touched, plus one (`lastInteractedSlot` starts at -1).
        return Some(match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
            Some(BlockEntity::ChiseledBookshelf(b)) => b.last_slot + 1,
            _ => 0,
        });
    }
    // Only read from the back: a bit for each of the three slots in use.
    let facing = info.properties.get("facing").and_then(|f| dir_from_name(f));
    if facing.map(Direction::opposite) != Some(direction) {
        return Some(0);
    }
    let Some(BlockEntity::Shelf(shelf)) = world.block_entities.get(&(pos.x, pos.y, pos.z)) else {
        return Some(0);
    };
    Some(
        shelf
            .items
            .iter()
            .enumerate()
            .map(|(slot, item)| i32::from(item.is_some()) << slot)
            .sum(),
    )
}

impl Game<'_> {
    pub(super) fn containers(&self) -> Containers<'_> {
        Containers {
            world: &self.world,
            behaviours: self.res.behaviours(),
            items: self.items(),
            registry: self.pack.assets.registry,
            brewing: Some(&self.stations().brewing),
            mobs: &self.mobs,
        }
    }
    pub(super) fn block_can_place(&self, pos: [i32; 3], slot: usize, stack: &ItemStack) -> bool {
        let containers = self.containers();
        let Some(view) = containers.view((pos[0], pos[1], pos[2]), false) else {
            return false;
        };
        view.kind == Kind::Shulker || containers.can_place(&view, slot, stack)
    }
    fn slot_mut_at(&mut self, view: &View, slot: usize) -> Option<&mut Option<ItemStack>> {
        let (pos, i) = view.locate(slot)?;
        let entity = self.world.block_entities.get_mut(&pos)?;
        // A mod's slots grow to its container's count when first used.
        if let BlockEntity::Mod(m) = entity
            && m.items.len() <= i
        {
            m.items.resize(view.len(), None);
        }
        entity.slots_mut().get_mut(i)
    }
    /// `BlockEntity.setChanged` for every block entity of a container.
    fn view_changed(&mut self, view: &View) {
        // A composter's containers change its block state instead.
        if let Kind::Composter(_) = view.kind {
            return;
        }
        for &(pos, _) in &view.parts {
            self.container_changed(pos);
            // `ShelfBlockEntity.setChanged()` (the no-argument override posts
            // `BLOCK_ACTIVATE`): a container writing a shelf's slot is heard.
            if matches!(
                self.world.block_entities.get(&pos),
                Some(BlockEntity::Shelf(_))
            ) {
                let state = self.world.block(pos.0, pos.1, pos.2);
                self.post_block_game_event(
                    "minecraft:block_activate",
                    [pos.0, pos.1, pos.2],
                    None,
                    Some(state),
                );
            }
        }
    }
    /// `BlockEntity.setChanged` at `pos`: comparators re-read the container.
    pub(super) fn container_changed(&mut self, pos: Pos) {
        if pos.1 == super::minecarts::CART_KEY_Y {
            return;
        }
        if mod_api::__private::has_scoped_hook(mod_api::HookId::ContentsChanged)
            && let Some(level) = self.native_level_here()
        {
            let at = BlockPos::new(pos.0, pos.1, pos.2);
            self.native_contents_changed(level, at, self.world.block(pos.0, pos.1, pos.2));
        }
        self.chiseled_bookshelf_synced(pos);
        if matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Shelf(_))
        ) {
            self.mark_block_entity_updated(pos);
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
        stack: ItemStack,
        slot: usize,
    ) -> Option<ItemStack> {
        let (rest, moved) = self.move_in_item(from_hopper, view, stack, slot);
        if moved {
            self.view_changed(view);
            // JukeboxBlockEntity.setTheItem through a hopper's or a dropper's
            // transfer: the disc it lands on starts playing.
            if view.kind == Kind::Jukebox {
                let (x, y, z) = view.parts[0].0;
                self.jukebox_inserted([x, y, z]);
            }
        }
        rest
    }
    /// `tryMoveInItem` before its `setChanged`: the remainder, and whether
    /// anything moved.
    fn move_in_item(
        &mut self,
        from_hopper: Option<Pos>,
        view: &View,
        mut stack: ItemStack,
        slot: usize,
    ) -> (Option<ItemStack>, bool) {
        let c = self.containers();
        if !c.can_place(view, slot, &stack) {
            return (Some(stack), false);
        }
        self.unpack_inventory_slot(view, slot);
        let was_empty = self.inventory_is_empty(view);
        let c = self.containers();
        let vanilla_max = c.max_stack(&stack);
        let override_limit = view
            .locate(slot)
            .and_then(|(pos, _)| c.world.container_stack_limit(pos));
        let restricted = override_limit.is_some_and(|limit| limit < vanilla_max);
        let max = override_limit.map_or(vanilla_max, |limit| vanilla_max.min(limit));
        if restricted && max == 0 {
            return (Some(stack), false);
        }

        let mut success = false;
        let mut filled_empty = false;
        let mut pot = if view.kind == Kind::Pot {
            let pos = view.parts[0].0;
            let Some(tag) = spill::saved_entity(&self.world, [pos.0, pos.1, pos.2])
                .or_else(|| self.pot_data([pos.0, pos.1, pos.2]))
            else {
                return (Some(stack), false);
            };
            let item = tag.get("item").and_then(save::item_from_nbt);
            Some((tag, item))
        } else {
            None
        };
        let target = if let Some((_, item)) = &mut pot {
            item
        } else {
            let Some(target) = self.slot_mut_at(view, slot) else {
                return (Some(stack), false);
            };
            target
        };
        // An empty stack a mod left in its slot: the slot is free (`isEmpty`).
        if target.as_ref().is_some_and(|s| s.count == 0) {
            *target = None;
        }
        match target {
            None => {
                // HopperBlockEntity.setItem limits the accepted stack, then
                // tryMoveInItem consumes the input even when it exceeded that limit.
                *target = Some(ItemStack {
                    count: stack.count.min(max),
                    ..stack.clone()
                });
                stack.count = if restricted { stack.count.saturating_sub(max) } else { 0 };
                success = true;
                filled_empty = true;
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
        if success && let Some((mut tag, item)) = pot {
            let pos = view.parts[0].0;
            if let Tag::Compound(map) = &mut tag {
                if let Some(item) = item {
                    map.insert("item".into(), save::item_to_nbt(&item));
                } else {
                    map.remove(&"item".into());
                }
            }
            spill::put_saved_entity(&mut self.world, [pos.0, pos.1, pos.2], tag);
            self.mark_block_entity_updated(pos);
        }
        // tryMoveInItem's setItem on an empty slot: AbstractFurnaceBlockEntity.setItem restarts
        // the cooking for a new input (a merge only grows the stack, and keeps it).
        let recipes = world_recipes!(self);
        if filled_empty
            && let Some((pos, 0)) = view.locate(slot)
            && let Some(BlockEntity::Furnace(furnace)) = self.world.block_entities.get_mut(&pos)
        {
            use horizon_client::furnace::FurnaceCooking;
            let input = furnace.items[0].take();
            furnace.set_input(&self.loop_context, recipes, input);
        }
        if success && was_empty && view.kind == Kind::Hopper {
            let dest = view.parts[0].0;
            let from_ticked = from_hopper.and_then(|p| self.hoppers.get(&p).map(|h| h.ticked));
            let state = self.hoppers.entry(dest).or_default();
            if state.cooldown <= 8 {
                let skip = i32::from(from_ticked.is_some_and(|t| state.ticked >= t));
                state.cooldown = 8 - skip;
            }
        }
        ((stack.count > 0).then_some(stack), success)
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
        let mut rest = Some(stack);
        for slot in view.slots_to_add_iter(face) {
            let Some(stack) = rest.take() else { break };
            rest = self.try_move_in_item(from_hopper, view, stack, slot);
        }
        rest
    }
    /// Remove one item from a slot (`removeItem(slot, 1)`).
    fn take_one(&mut self, view: &View, slot: usize) -> Option<ItemStack> {
        if view.kind == Kind::Pot {
            return self.pot_take_one(view.parts[0].0);
        }
        let target = self.slot_mut_at(view, slot)?;
        let current = target.as_mut().filter(|s| s.count > 0)?;
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
    /// `ContainerSingleItem.removeItem(0, 1)` on a decorated pot's saved slot.
    fn pot_take_one(&mut self, pos: Pos) -> Option<ItemStack> {
        let mut tag = spill::saved_entity(&self.world, [pos.0, pos.1, pos.2])?;
        let mut item = tag.get("item").and_then(save::item_from_nbt)?;
        if item.count == 0 {
            return None;
        }
        let one = ItemStack {
            count: 1,
            ..item.clone()
        };
        item.count -= 1;
        self.pot_store(pos, &mut tag, (item.count > 0).then_some(item));
        Some(one)
    }
    /// Undo `pot_take_one` after a failed move.
    fn pot_put_back(&mut self, pos: Pos, one: ItemStack) {
        let Some(mut tag) = spill::saved_entity(&self.world, [pos.0, pos.1, pos.2]) else {
            return;
        };
        let item = match tag.get("item").and_then(save::item_from_nbt) {
            Some(mut current) => {
                current.count += one.count;
                current
            }
            None => one,
        };
        self.pot_store(pos, &mut tag, Some(item));
    }
    /// Write a pot's saved item slot, keeping the tag's other metadata.
    fn pot_store(&mut self, pos: Pos, tag: &mut Tag, item: Option<ItemStack>) {
        if let Tag::Compound(map) = tag {
            match item {
                Some(item) => {
                    map.insert("item".into(), save::item_to_nbt(&item));
                }
                None => {
                    map.remove(&"item".into());
                }
            }
        }
        spill::put_saved_entity(&mut self.world, [pos.0, pos.1, pos.2], tag.clone());
        self.mark_block_entity_updated(pos);
    }
    /// Undo `take_one` after a failed move.
    fn put_back(&mut self, view: &View, slot: usize, one: ItemStack) {
        if view.kind == Kind::Pot {
            self.pot_put_back(view.parts[0].0, one);
            return;
        }
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
        let time = self.world.game_time;
        // Every hopper keeps its cooldown and the tick it last ticked
        // (`HopperBlockEntity.tickedGameTime`), idle ones too: a hopper that
        // receives an item reads it for its cooldown (8, or 7 once it ticked).
        let state = self.hoppers.entry(pos).or_default();
        state.cooldown = state.cooldown.wrapping_sub(1);
        state.ticked = time;
        if state.cooldown > 0 {
            return;
        }
        state.cooldown = 0;
        // Most ticks only advance the cooldown: the block is checked when the
        // hopper is about to move, not on every tick it waits.
        let hopper = horizon_client::block_entity_types::factory_block(
            &self
                .res
                .facts()
                .get(self.world.block(pos.0, pos.1, pos.2))
                .name,
        ) == "minecraft:hopper";
        if !hopper {
            self.hoppers.remove(&pos);
            return;
        }
        let state = self.world.block(pos.0, pos.1, pos.2);
        self.hopper_try_move_items_with_state(pos, state);
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
        self.hopper_try_move_items_with_state(pos, state);
    }
    fn hopper_try_move_items_with_state(&mut self, pos: Pos, state: BlockStateId) {
        let c = self.containers();
        let enabled = c.prop(state, "enabled") != Some("false");
        let Some(view) = c.view(pos, false) else {
            return;
        };
        if self.hoppers.get(&pos).is_some_and(|h| h.cooldown > 0) || !enabled {
            return;
        }
        self.native_prepare_hopper_transfer([pos.0, pos.1, pos.2]);
        self.unpack_loot(super::container_loot::LootTarget::Block(pos), None);
        let c = self.containers();
        let mut changed = false;
        if !c.is_empty(&view) {
            changed = self.hopper_eject(pos, &view);
        }
        if !self.hopper_is_stacked(&view) {
            changed |= self.hopper_suck(pos, &view);
        }
        if changed {
            self.hoppers.entry(pos).or_default().cooldown = 8;
            self.container_changed(pos);
        }
    }
    /// `HopperBlockEntity.entityInside` for the item entities that moved this
    /// tick: an item whose box meets `Hopper.SUCK_AABB` runs `tryMoveItems`
    /// with that item as the pickup, in the entity phase, before the hopper's
    /// own tick counts its cooldown down.
    pub(super) fn hoppers_catch_items(&mut self, original_moves: &FxHashMap<u32, Vec3>) {
        let mut i = 0;
        while i < self.dropped.len() {
            let item = &self.dropped[i];
            // `ItemEntity.tick` moves 1 of 4 ticks while resting;
            // `checkInsideBlocks` only follows a move.
            let moved = item.position != item.previous
                || item.ticks.wrapping_add(item.id).is_multiple_of(4);
            if !moved || item.stack.count == 0 {
                i += 1;
                continue;
            }
            let bounds = item.bounds();
            let movement = MovementRecord {
                from: item.previous,
                to: item.position,
                original: original_moves.get(&item.id).copied(),
            };
            let epsilon = f64::from(1.0e-5_f32);
            let deflation = Vec3::new(epsilon, epsilon, epsilon);
            // Axis segments and the final-box retry share the visited hoppers.
            let mut tried = Vec::new();
            let mut visit = |pos: BlockPos| {
                let state = self.world.block(pos.x, pos.y, pos.z);
                // Hopper.SUCK_AABB is tested against the item's actual box,
                // while the block visitor uses the deflated movement box.
                let suction = Aabb::new(Vec3::new(0.0, 11.0 / 16.0, 0.0), Vec3::new(1.0, 2.0, 1.0))
                    .move_by(Vec3::from_block(pos));
                if horizon_client::block_entity_types::factory_block(
                    &self.res.facts().get(state).name,
                ) == "minecraft:hopper"
                    && bounds.intersects(suction)
                    && !tried.contains(&pos)
                {
                    tried.push(pos);
                    return !self.hopper_take_item((pos.x, pos.y, pos.z), i);
                }
                true
            };
            let still_dropped = sweep_movement_segments(movement, |from, to, budget| {
                let segment_box = bounds.move_by(to.subtract(movement.to)).deflate(deflation);
                let mut iterations = 0;
                let mut taken = false;
                let _ = sweep_intersected_blocks(from, to, segment_box, 1_000_000, |pos, step| {
                    if step >= budget {
                        return false;
                    }
                    iterations = step;
                    taken = !visit(pos);
                    !taken
                });
                if taken { Err(()) } else { Ok(iterations + 1) }
            })
            .is_ok();
            if still_dropped {
                i += 1;
            }
        }
    }
    /// `tryMoveItems(…, () -> addItem(hopper, itemEntity))`; true when the
    /// item entity was taken whole.
    fn hopper_take_item(&mut self, pos: Pos, i: usize) -> bool {
        if self.hoppers.get(&pos).is_some_and(|h| h.cooldown > 0) {
            return false;
        }
        let state = self.world.block(pos.0, pos.1, pos.2);
        let c = self.containers();
        if c.prop(state, "enabled") == Some("false") {
            return false;
        }
        let Some(view) = c.view(pos, false) else {
            return false;
        };
        self.unpack_loot(super::container_loot::LootTarget::Block(pos), None);
        let c = self.containers();
        let mut changed = false;
        if !c.is_empty(&view) {
            changed = self.hopper_eject(pos, &view);
        }
        let mut taken = false;
        if !self.hopper_is_stacked(&view) {
            let stack = self.dropped[i].stack.clone();
            let allowed = self.with_level(|_, level, _| {
                level.allow_hopper_pickup_item(
                    BlockPos::new(pos.0, pos.1, pos.2),
                    &stack.item,
                    stack.count,
                )
            });
            match allowed
                .then(|| self.add_item(None, &view, stack, None))
                .flatten()
            {
                None if !allowed => {}
                None => {
                    self.remove_dropped(i);
                    taken = true;
                    changed = true;
                }
                Some(rest) => self.dropped[i].stack = rest,
            }
        }
        if changed {
            self.hoppers.entry(pos).or_default().cooldown = 8;
            self.container_changed(pos);
        }
        taken
    }
    /// A mod's item handler through `side`. Vanilla containers and container providers keep
    /// their direct automation path even when a capability overrides their public storage.
    pub(super) fn item_part_at(
        &mut self,
        pos: Pos,
        side: Direction,
    ) -> Option<Capability<dyn ResourceHandler<ItemResource>>> {
        let at = BlockSide {
            level: self.native_level_here()?,
            pos: [pos.0, pos.1, pos.2],
            side: Some(side),
        };
        let providers = mod_api::block_providers(&mod_api::ITEMS, self, at)?;
        if self.vanilla_automation_container(pos) {
            return None;
        }
        let game = self.container_size(at).is_some();
        providers.ask(self, game)
    }
    /// Offers `stack` to a capability part and, in the same transaction, `take`s what it took
    /// from where it came, after the part's own writes, so a part handing it on back into the
    /// source loses nothing: how many went. A part taking none, a part answering more than it
    /// was offered (a mod bug) and a failed `take` undo the move (none went).
    pub(super) fn put_into_part(
        &mut self,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        at: [i32; 3],
        stack: &ItemStack,
        take: impl FnOnce(&mut Self, u32) -> Option<()>,
    ) -> u32 {
        use super::api_values::TryIntoApi;
        let Ok(snapshot) = stack.try_into_api() else {
            return 0;
        };
        let resource = ItemResource::of(&snapshot);
        mod_api::run_transaction(self, |game, tx| {
            let host: &mut dyn mod_api::ServerHost = game;
            let went = part.insert_any(host, tx, &resource, stack.count);
            match part.checked(went, stack.count, &at) {
                Some(0) | None => Err(()),
                Some(went) => take(game, went).map(|()| went).ok_or(()),
            }
        })
        .unwrap_or(0)
    }
    /// `InventoryMoveItemEvent` for `item` going from `source` to `destination`, where one end
    /// is a capability part: whether a mod canceled the move.
    fn part_move_canceled(&mut self, source: [i32; 3], destination: [i32; 3], item: &str) -> bool {
        let mut cancel = false;
        self.native_inventory_move_item(source, destination, item, &mut cancel);
        cancel
    }
    /// `HopperBlockEntity.ejectItems`: one root attempt across source slots, restoring each
    /// refused slot directly. Only the caller's final cooldown mark changes the hopper.
    fn eject_into_part(
        &mut self,
        hopper: &View,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        at: [i32; 3],
    ) -> bool {
        if (0..part.size(self)).all(|slot| {
            part.resource(self, slot).is_some_and(|resource| {
                part.amount(self, slot) >= part.capacity(self, slot, &resource)
            })
        }) {
            return false;
        }
        let from = self.container_world_pos(hopper);
        mod_api::run_transaction(self, |game, tx| {
            use super::api_values::TryIntoApi;
            for slot in 0..hopper.len() {
                let Some(snapshot) = game
                    .containers()
                    .get(hopper, slot)
                    .and_then(|stack| stack.try_into_api().ok())
                else {
                    continue;
                };
                let resource = ItemResource::of(&snapshot);
                let item = mod_api::__private::item_key_id(snapshot.item()).unwrap();
                if game.part_move_canceled(from, at, item) {
                    continue;
                }
                let (source, index) = hopper.locate(slot).ok_or(())?;
                game.journal_block_entity_slot(source, index);
                let Some(mut one) = game.take_one(hopper, slot) else {
                    continue;
                };
                let went = part.insert_any(game, tx, &resource, 1);
                let accepted = part.checked(went, 1, &at);
                if accepted == Some(1) {
                    return Ok(());
                }
                one.count = snapshot.count();
                *game.slot_mut_at(hopper, slot).ok_or(())? = Some(one);
                accepted.ok_or(())?;
            }
            Err(())
        })
        .is_ok()
    }
    /// `HopperBlockEntity.suckInItems`: extract before choosing the first receptive hopper
    /// slot. Each nonempty storage index gets its own commit or abort.
    fn suck_from_part(
        &mut self,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        at: [i32; 3],
        hopper: &View,
    ) -> bool {
        if self.hopper_is_stacked(hopper) {
            return false;
        }
        let into = self.container_world_pos(hopper);
        for slot in 0..part.size(self) {
            let Some(resource) = part.resource(self, slot) else {
                continue;
            };
            use super::api_values::TryIntoEngine;
            let Ok(one) = resource.as_stack().try_into_engine(self.items()) else {
                continue;
            };
            if part.amount(self, slot) == 0 || self.part_move_canceled(at, into, &one.item) {
                continue;
            }
            let moved = mod_api::run_transaction(self, |game, tx| {
                let host: &mut dyn mod_api::ServerHost = game;
                let came = part.extract(host, tx, slot, &resource, 1);
                if part.checked(came, 1, &at) != Some(1) {
                    return Err(());
                }
                let target = (0..hopper.len())
                    .find(|&slot| game.containers().fits(hopper, slot, &one) > 0)
                    .ok_or(())?;
                let (pos, slot) = hopper.locate(target).ok_or(())?;
                game.journal_block_entity_slot(pos, slot);
                game.try_move_in_item(None, hopper, one, target)
                    .map_or(Ok(()), |_| Err(()))
            });
            if moved.is_ok() {
                return true;
            }
        }
        false
    }
    /// `HopperBlockEntity.ejectItems`: into the block container in front, else
    /// a container entity there (`getContainerAt`). A capability part in front takes the
    /// container's place.
    fn hopper_eject(&mut self, pos: Pos, hopper: &View) -> bool {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let Some(facing) = self
            .containers()
            .prop(state, "facing")
            .and_then(dir_from_name)
        else {
            return false;
        };
        let front = offset(pos, facing);
        let face = facing.opposite();
        if let Some(part) = self.item_part_at(front, face) {
            let at = [front.0, front.1, front.2];
            self.native_hopper_inventory_search([pos.0, pos.1, pos.2], Some(at), None);
            return self.eject_into_part(hopper, &part, at);
        }
        if let Some(target) = self.containers().hopper_insertion_view(front) {
            self.native_hopper_inventory_search(
                [pos.0, pos.1, pos.2],
                Some([front.0, front.1, front.2]),
                None,
            );
            return self.eject_into(pos, hopper, &target, face);
        }
        let Some(found) = self.automation_entity_at(
            [
                f64::from(front.0) + 0.5,
                f64::from(front.1) + 0.5,
                f64::from(front.2) + 0.5,
            ],
            face,
        ) else {
            return false;
        };
        match found {
            AutomationEntity::Container(id) => {
                self.native_hopper_inventory_search([pos.0, pos.1, pos.2], None, Some(id));
                self.with_entity_container(id, |g, target| g.eject_into(pos, hopper, target, face))
                    .unwrap_or(false)
            }
            AutomationEntity::Part(entity, part) => {
                self.native_hopper_inventory_search([pos.0, pos.1, pos.2], None, Some(entity.id()));
                self.eject_into_part(hopper, &part, [front.0, front.1, front.2])
            }
        }
    }
    /// Where a container is in the world for the mods: its block, or the block a chest
    /// boat or minecart is in (its view sits under a placeholder key).
    fn container_world_pos(&self, view: &View) -> [i32; 3] {
        let (x, y, z) = view.parts[0].0;
        if x == i32::MAX - 15 && y == super::minecarts::CART_KEY_Y {
            let id = z as u32;
            let at = self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.position)
                .or_else(|| self.boats.iter().find(|b| b.id == id).map(|b| b.position));
            if let Some(p) = at {
                return [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            }
        }
        [x, y, z]
    }
    /// `InventoryMoveItemEvent` for the item in `slot` of `holder` going from `source` to
    /// `destination`: whether a mod canceled the move. Nothing is looked up with no mod loaded.
    fn hopper_move_canceled(
        &self,
        holder: &View,
        slot: usize,
        source: &View,
        destination: &View,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::InventoryMoveItem) {
            return false;
        }
        let containers = self.containers();
        let Some(stack) = containers.get(holder, slot) else {
            return false;
        };
        let mut cancel = false;
        self.native_inventory_move_item(
            self.container_world_pos(source),
            self.container_world_pos(destination),
            &stack.item,
            &mut cancel,
        );
        cancel
    }
    /// The rest of `HopperBlockEntity.ejectItems`: one item into `target`.
    fn eject_into(&mut self, pos: Pos, hopper: &View, target: &View, face: Direction) -> bool {
        self.unpack_inventory_slot(hopper, 0);
        if self.inventory_is_full(target, face) {
            return false;
        }
        for slot in 0..hopper.len() {
            if self.hopper_move_canceled(hopper, slot, hopper, target) {
                continue;
            }
            let Some(one) = self.take_one(hopper, slot) else {
                continue;
            };
            match self.add_item(Some(pos), target, one, Some(face)) {
                None => {
                    self.view_changed(target);
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
        let at = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 1.5,
            f64::from(pos.2) + 0.5,
        ];
        if let Some(took) = self.suck_from_container(above, at, hopper) {
            return took;
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
        if self.dropped.is_empty() {
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
        self.suck_item_entities(Some(pos), hopper, min, max)
    }
    /// The container half of `HopperBlockEntity.suckInItems`: one item from the
    /// container at `above`, through its bottom face; with no block container
    /// there, from a container entity whose box meets the one around `at`
    /// (`getSourceContainerOrHandler`). `None` when there is no container. A capability part
    /// above takes the container's place.
    fn suck_from_container(&mut self, above: Pos, at: [f64; 3], hopper: &View) -> Option<bool> {
        let (x, y, z) = hopper.parts[0].0;
        if let Some(part) = self.item_part_at(above, Direction::Down) {
            let above = [above.0, above.1, above.2];
            self.native_hopper_inventory_search([x, y, z], Some(above), None);
            return Some(self.suck_from_part(&part, above, hopper));
        }
        let c = self.containers();
        let Some(source) = c.hopper_insertion_view(above) else {
            let found = self.automation_entity_at(at, Direction::Down)?;
            return match found {
                AutomationEntity::Container(id) => {
                    self.native_hopper_inventory_search([x, y, z], None, Some(id));
                    self.with_entity_container(id, |g, source| {
                        g.suck_from_slots(None, source, hopper)
                    })
                }
                AutomationEntity::Part(entity, part) => {
                    self.native_hopper_inventory_search([x, y, z], None, Some(entity.id()));
                    Some(self.suck_from_part(&part, [above.0, above.1, above.2], hopper))
                }
            };
        };
        self.native_hopper_inventory_search([x, y, z], Some([above.0, above.1, above.2]), None);
        if let Kind::Composter(level) = source.kind {
            if level != 8 {
                return Some(false);
            }
            // tryTakeInItemFromSlot on the OutputContainer: removeItem calls
            // its setChanged, which empties the composter, and so does the
            // second setChanged after the move or, when the hopper has no
            // room, the setItem putting the bone meal back into the detached
            // container (whose bone meal is lost with it).
            let bone_meal = bone_meal_stack().clone();
            if mod_api::__private::listened(mod_api::HookId::InventoryMoveItem) {
                let mut cancel = false;
                self.native_inventory_move_item(
                    [above.0, above.1, above.2],
                    self.container_world_pos(hopper),
                    &bone_meal.item,
                    &mut cancel,
                );
                if cancel {
                    return Some(false);
                }
            }
            let at = BlockPos::new(above.0, above.1, above.2);
            self.with_level(|b, level, ticks| b.composter_empty(level, ticks, at));
            let moved = self.add_item(None, hopper, bone_meal, None).is_none();
            self.with_level(|b, level, ticks| b.composter_empty(level, ticks, at));
            self.world.modified.insert((above.0 >> 4, above.2 >> 4));
            return Some(moved);
        }
        let source_hopper = (source.kind == Kind::Hopper).then_some(above);
        Some(self.suck_from_slots(source_hopper, &source, hopper))
    }
    /// `HopperBlockEntity.tryTakeInItemFromSlot` over the `DOWN` face of `source`.
    fn suck_from_slots(
        &mut self,
        source_hopper: Option<Pos>,
        source: &View,
        hopper: &View,
    ) -> bool {
        let mut prepared = None;
        for slot in source.slots_for_face_iter(Some(Direction::Down)) {
            self.unpack_next_inventory_part(source, slot, &mut prepared);
            {
                let c = self.containers();
                let Some(stack) = c.insertion_item(source, slot).map(Cow::into_owned) else {
                    continue;
                };
                if !c.can_take_into(source, slot, &stack, Direction::Down, hopper) {
                    continue;
                }
            }
            if self.hopper_move_canceled(source, slot, source, hopper) {
                continue;
            }
            let Some(one) = self.take_one(source, slot) else {
                continue;
            };
            match self.add_item(source_hopper, hopper, one, None) {
                None => {
                    self.view_changed(source);
                    // JukeboxBlockEntity.removeItem -> setTheItem(EMPTY): a
                    // disc a hopper takes out stops the song.
                    if source.kind == Kind::Jukebox {
                        let (x, y, z) = source.parts[0].0;
                        self.jukebox_taken([x, y, z]);
                    }
                    return true;
                }
                Some(rest) => {
                    self.put_back(source, slot, rest);
                }
            }
        }
        false
    }
    /// The alive container entities (`EntitySelector.CONTAINER_ENTITY_SELECTOR`: chest and
    /// hopper minecarts, chest boats) whose box meets the 1x1x1 box around `at`.
    fn container_entities_at(&self, at: [f64; 3]) -> Vec<u32> {
        let meets = |b: horizon_core::geometry::Aabb| {
            let (lo, hi) = (b.min_position(), b.max_position());
            lo.x < at[0] + 0.5
                && hi.x > at[0] - 0.5
                && lo.y < at[1] + 0.5
                && hi.y > at[1] - 0.5
                && lo.z < at[2] + 0.5
                && hi.z > at[2] - 0.5
        };
        let mut found: Vec<u32> = self
            .carts
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    super::minecarts::CartKind::Chest | super::minecarts::CartKind::Hopper
                ) && meets(c.bounds())
            })
            .map(|c| c.id)
            .collect();
        found.extend(
            self.boats
                .iter()
                .filter(|b| b.chest && meets(b.bounds()))
                .map(|b| b.id),
        );
        found
    }
    /// The index `RandomSource.nextInt(len)` picks of `len` candidates: the level's one draw,
    /// none for no candidate.
    fn pick_index(&mut self, len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        let pick = self.world.random.next_int_bound(len as i32).unwrap_or(0);
        Some(pick as usize)
    }
    /// The entity item automation reaches around `at` from `side`, through `ENTITY_AUTOMATION`: one of
    /// the container entities the game has there, and, once a mod registered a provider for it,
    /// the entities whose mod provider answers it, picked at random as `getEntityContainer` picks, the one draw.
    /// Vanilla container entities always keep their direct path, even with a provider.
    fn automation_entity_at(&mut self, at: [f64; 3], side: Direction) -> Option<AutomationEntity> {
        let vanilla = self.container_entities_at(at);
        if !mod_api::has_entity_providers(&ENTITY_AUTOMATION) {
            let pick = self.pick_index(vanilla.len())?;
            return Some(AutomationEntity::Container(vanilla[pick]));
        }
        let mut found: Vec<AutomationEntity> = Vec::new();
        let context = Some(side);
        let ask = |game: &mut Self, entity: EntityRef, has_game_answer: bool| {
            let providers = mod_api::entity_providers(&ENTITY_AUTOMATION, &*game, entity, context)?;
            providers.ask(game, has_game_answer)
        };
        found.extend(vanilla.iter().copied().map(AutomationEntity::Container));
        if let Some(level) = self.native_level_here() {
            let min = [at[0] - 0.5, at[1] - 0.5, at[2] - 0.5];
            let max = [at[0] + 0.5, at[1] + 0.5, at[2] + 0.5];
            for entity in self.level_entities_in(level, min, max).unwrap_or_default() {
                if !vanilla.contains(&entity.id())
                    && let Some(part) = ask(self, entity, false)
                {
                    found.push(AutomationEntity::Part(entity, part));
                }
            }
        }
        let pick = self.pick_index(found.len())?;
        Some(found.swap_remove(pick))
    }
    /// Runs `f` on the block container at `pos`, or one entity there, with providers taking
    /// the game's slots' place when registered ahead of them.
    fn with_container_at<R>(
        &mut self,
        pos: Pos,
        side: Direction,
        f: impl FnOnce(&mut Self, ContainerTarget<'_>) -> R,
    ) -> Option<R> {
        if let Some(part) = self.item_part_at(pos, side) {
            return Some(f(self, ContainerTarget::Part(part)));
        }
        if let Some(view) = self.containers().hopper_insertion_view(pos) {
            return Some(f(self, ContainerTarget::View(&view)));
        }
        let centre = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        ];
        match self.automation_entity_at(centre, side)? {
            AutomationEntity::Container(id) => {
                self.with_entity_container(id, |game, view| f(game, ContainerTarget::View(view)))
            }
            AutomationEntity::Part(_, part) => Some(f(self, ContainerTarget::Part(part))),
        }
    }
    /// Runs `f` on a container entity's items as a container, under the
    /// placeholder key its menu uses (the open menu's copy when it is open),
    /// then hands the items back to the entity.
    fn with_entity_container<R>(
        &mut self,
        id: u32,
        f: impl FnOnce(&mut Self, &View) -> R,
    ) -> Option<R> {
        let mut items = if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            self.unpack_loot(super::container_loot::LootTarget::Cart(id), None);
            self.carts.update(i, |c| std::mem::take(&mut c.items))
        } else {
            let i = self.boats.iter().position(|b| b.id == id)?;
            self.unpack_boat_loot(i, false);
            self.boats.update(i, |b| std::mem::take(&mut b.items))
        };
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, id as i32);
        let (view, open) = self.lend_items(key, &mut items);
        let result = f(self, &view);
        self.reclaim_items(key, open, &mut items);
        if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            self.carts.update(i, |c| c.items = items);
        } else if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            self.boats.update(i, |b| b.items = items);
        }
        Some(result)
    }
    /// A vehicle's slots as a container under its placeholder `key`. When its
    /// menu is open the placeholder is the live copy and stays as it is.
    /// Returns the view and whether the menu was open.
    fn lend_items(&mut self, key: Pos, items: &mut Vec<Option<ItemStack>>) -> (View, bool) {
        let open = self.world.block_entities.contains_key(&key);
        if !open {
            let slots = BlockEntity::Chest(std::mem::take(items));
            self.world.block_entities.insert(key, slots);
        }
        let size = self
            .world
            .block_entities
            .get(&key)
            .map_or(0, |e| e.slots().len());
        let view = View {
            parts: vec![(key, size)],
            kind: Kind::Generic,
            container: None,
        };
        (view, open)
    }
    /// Gives `lend_items`' slots back to the vehicle, so its own items stay
    /// the one source of truth: a menu copy that is still around (open, or
    /// left by a guest's close) gets the changed items copied to `items`.
    fn reclaim_items(&mut self, key: Pos, open: bool, items: &mut Vec<Option<ItemStack>>) {
        let slots = if open {
            self.world.block_entities.get(&key).cloned()
        } else {
            self.world.block_entities.remove(&key)
        };
        if let Some(BlockEntity::Chest(slots)) = slots {
            *items = slots;
        }
    }
    /// The item half of `HopperBlockEntity.suckInItems` (`getItemsAtAndAbove`):
    /// adds the item entities that meet the box, until one is taken whole.
    fn suck_item_entities(
        &mut self,
        hopper_pos: Option<Pos>,
        hopper: &View,
        min: [f64; 3],
        max: [f64; 3],
    ) -> bool {
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
                let allowed = hopper_pos.is_none_or(|pos| {
                    self.with_level(|_, level, _| {
                        level.allow_hopper_pickup_item(
                            BlockPos::new(pos.0, pos.1, pos.2),
                            &stack.item,
                            stack.count,
                        )
                    })
                });
                if !allowed {
                    i += 1;
                    continue;
                }
                match self.add_item(None, hopper, stack, None) {
                    None => {
                        self.remove_dropped(i);
                        return true;
                    }
                    Some(rest) => self.dropped[i].stack = rest,
                }
            }
            i += 1;
        }
        false
    }
    /// `suck_item_entities` for a hopper minecart, which first asks the item
    /// columns whether any item's box reaches the columns of the box: an item
    /// that meets it reaches one of them. The columns follow the item list's
    /// stamp, so any write since they were built (an item's move, an
    /// explosion's push, a command, a pickup) makes them scan, then rebuild.
    fn cart_suck_items(&mut self, hopper: &View, min: [f64; 3], max: [f64; 3]) -> bool {
        if !self
            .pickup_columns
            .get_or_insert_with(Default::default)
            .may_reach_area(
                &self.dropped,
                (min[0].floor() as i32, max[0].floor() as i32),
                (min[2].floor() as i32, max[2].floor() as i32),
            )
        {
            return false;
        }
        self.suck_item_entities(None, hopper, min, max)
    }
    /// `MinecartHopper.tick`'s `tryConsumeItems` for whichever path ticks the
    /// cart. `ServerLevel.tickNonPassenger` ticks the vehicle, then its
    /// passengers, so a ridden cart pulls too; Horizon ticks that one in its
    /// rider's turn (`tick_cart_riding`). `alive` is `isAlive` after the move.
    pub(super) fn hopper_cart_tick(&mut self, cart: &mut super::minecarts::Minecart, alive: bool) {
        if alive && cart.kind == super::minecarts::CartKind::Hopper && cart.enabled {
            self.hopper_cart_suck(cart);
        }
    }
    /// `MinecartHopper.tick`'s `tryConsumeItems` (`suckInItems`) for an enabled
    /// hopper minecart: one item from the container above, else the item
    /// entities over it or touching it, each tick without a cooldown.
    /// `consumedItemThisFrame` in 26.3 caps a tick at one item: the tick tries
    /// once, and `makeStepAlongTrack` may try again, but only one succeeds.
    pub(super) fn hopper_cart_suck(&mut self, cart: &mut super::minecarts::Minecart) {
        if !self.unpack_ticked_cart_loot(cart) {
            return;
        }
        // The cart's slots are edited as a container under the placeholder key
        // its menu uses; when that menu is open the placeholder is the live copy.
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, cart.id as i32);
        let (view, open) = self.lend_items(key, &mut cart.items);
        let p = cart.position;
        // getLevelY is the cart's y + 0.5, so the container is the block at
        // y + 1.5, and Hopper.SUCK_AABB (moved by getLevel - 0.5) starts 11/16
        // above the cart.
        let above = (
            p.x.floor() as i32,
            (p.y + 1.5).floor() as i32,
            p.z.floor() as i32,
        );
        let at = [p.x, p.y + 1.5, p.z];
        let took = match self.suck_from_container(above, at, &view) {
            Some(took) => took,
            None => self.cart_suck_items(
                &view,
                [p.x - 0.5, p.y + 11.0 / 16.0, p.z - 0.5],
                [p.x + 0.5, p.y + 2.0, p.z + 0.5],
            ),
        };
        if !took {
            // The cart's own box, inflated by 0.25 sideways.
            let b = cart.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            self.cart_suck_items(
                &view,
                [lo.x - 0.25, lo.y, lo.z - 0.25],
                [hi.x + 0.25, hi.y, hi.z + 0.25],
            );
        }
        self.reclaim_items(key, open, &mut cart.items);
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
        self.unpack_loot(super::container_loot::LootTarget::Block(key), None);
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
            let side = facing.opposite();
            let inserted = self.with_container_at(target, side, |game, into| match into {
                ContainerTarget::View(into) => {
                    let one = ItemStack {
                        count: 1,
                        ..stack.clone()
                    };
                    let inserted = game.add_item(None, into, one, Some(side)).is_none();
                    if inserted {
                        game.view_changed(into);
                    }
                    ControlFlow::Continue(inserted)
                }
                ContainerTarget::Part(part) => {
                    let at = [target.0, target.1, target.2];
                    game.dropper_into_part(&part, &stack, at, &view, slot);
                    ControlFlow::Break(())
                }
            });
            match inserted {
                Some(ControlFlow::Break(())) => {
                    self.container_changed(key);
                    return Ok(());
                }
                Some(ControlFlow::Continue(inserted)) => Some(ItemStack {
                    count: stack.count - u32::from(inserted),
                    ..stack
                })
                .filter(|s| s.count > 0),
                None => self.dispense_default_in_level(key, facing, stack),
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
    pub(super) fn machine_layout_selector(&self) -> Option<bool> {
        self.machine_screen().map(|s| s == Screen::Hopper)
    }
    fn machine_screen(&self) -> Option<Screen> {
        let key = self.container?;
        // A mod's screen over a hopper or dispenser menu lays out its own slots.
        if self.native_screen_inventory_y().is_some() {
            return None;
        }
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
        if let Some(height) = self.native_screen_height() {
            return height;
        }
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
}

#[cfg(test)]
mod brewing_fuel_transfer_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::brewing::BrewingStand;
    use horizon_client::entities::ItemEntity;

    use mod_api::HostTransaction;

    struct PrepareProbe {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        fill: bool,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PrepareProbe {
        fn prepare_hopper_transfer(
            &self,
            host: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::PrepareHopperTransferEvent,
        ) {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fill {
                let level = host.level("minecraft:overworld").unwrap();
                let items = level
                    .capability(host, &mod_api::ITEMS, [8, 64, 8], None)
                    .unwrap();
                let stack = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stone").unwrap(),
                    1,
                )
                .unwrap();
                let resource = mod_api::ItemResource::of(&stack);
                host.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 0, &resource, 1), 1);
                    Ok::<_, ()>(())
                })
                .unwrap();
            }
        }
    }

    fn check_prepare_transfer(fill: bool) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "prepare_probe",
            Box::new(PrepareProbe {
                calls: calls.clone(),
                fill,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                let pos = (8, 64, 8);
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:hopper")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Chest(vec![None; HOPPER_SLOTS]));
                if fill {
                    let chest = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:chest")
                        .unwrap();
                    game.world.set(8, 63, 8, chest).unwrap();
                    game.world
                        .block_entities
                        .insert((8, 63, 8), BlockEntity::Chest(vec![None; 27]));
                }
                game.hopper_try_move_items(pos);
                assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
                if fill {
                    let containers = game.containers();
                    let target = containers.view((8, 63, 8), false).unwrap();
                    assert_eq!(
                        containers.get(&target, 0).unwrap().item.as_ref(),
                        "minecraft:stone"
                    );
                    assert_eq!(containers.get(&target, 0).unwrap().count, 1);
                    assert!(containers.is_empty(&containers.view(pos, false).unwrap()));
                    assert_eq!(game.hoppers[&pos].cooldown, 8);
                }
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn empty_hopper_dispatches_prepare_transfer() {
        check_prepare_transfer(false);
    }

    #[test]
    #[ignore = "daily"]
    fn prepare_transfer_inventory_write_is_ejected() {
        check_prepare_transfer(true);
    }

    /// `HopperBlockEntity.entityInside` uses `Hopper.SUCK_AABB`, the full-width
    /// column, not the old 12-wide bowl: with a solid block above (so the
    /// block-tick pickup refuses), an item on the outer rim still goes in.
    #[test]
    #[ignore = "daily"]
    fn a_hopper_catches_an_item_on_its_outer_rim() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let (x, y, z) = (8, 64, 8);
            let hopper = game
                .pack
                .assets
                .registry
                .default_state("minecraft:hopper")
                .unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(x, y, z, hopper).unwrap();
            game.world.set(x, y + 1, z, stone).unwrap();
            game.hoppers
                .insert((x, y, z), super::HopperState::default());
            game.world
                .block_entities
                .insert((x, y, z), BlockEntity::Chest(vec![None; 5]));
            let mut item = ItemEntity::new(
                2,
                ItemStack::new("minecraft:stone", 1),
                Vec3::new(f64::from(x) - 0.05, f64::from(y) + 0.7, f64::from(z) + 0.5),
                Vec3::ZERO,
                0.0,
            );
            item.previous = Vec3::new(f64::from(x) - 0.1, f64::from(y) + 0.7, f64::from(z) + 0.5);
            game.dropped.push(item);
            game.hoppers_catch_items(&FxHashMap::default());
            assert!(game.dropped.is_empty(), "the rim item was not taken");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_enabled_hopper_catches_an_item_overlapping_a_disabled_hopper() {
        check_adjacent_hopper_pickup(false, 8.93);
    }

    #[test]
    #[ignore = "daily"]
    fn a_westbound_item_between_hoppers_goes_into_the_east_hopper() {
        check_adjacent_hopper_pickup(true, 9.05);
    }

    #[test]
    #[ignore = "daily"]
    fn a_rising_item_clears_the_hoppers_cell_before_crossing_its_edge() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let hopper = game
                .pack
                .assets
                .registry
                .default_state("minecraft:hopper")
                .unwrap();
            let cases = [
                (2, Vec3::new(8.75, 64.8, 8.5), Vec3::new(0.24, 0.25, 0.0)),
                // A straight sweep visits the hopper cell in this case.
                (3, Vec3::new(8.85, 64.65, 10.5), Vec3::new(0.14, 0.4, 0.0)),
            ];
            for (id, start, velocity) in cases {
                let pos = (9, 64, start.z as i32);
                game.world.set(pos.0, pos.1, pos.2, hopper).unwrap();
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Chest(vec![None; HOPPER_SLOTS]));
                let mut item = ItemEntity::new(
                    id,
                    ItemStack::new("minecraft:stone", 1),
                    start,
                    velocity,
                    0.0,
                );
                item.no_gravity = true;
                game.ticked_entities_through = id;
                game.dropped.push(item);
            }

            let original_moves = game.tick_items();
            for (id, start, velocity) in cases {
                assert_eq!(original_moves.get(&id), Some(&velocity));
                let item = game.dropped.iter().find(|item| item.id == id).unwrap();
                assert_eq!(item.position, start.add_vector(velocity));
            }
            // Y resolves first, so the item's box clears y=65 before x enters the hopper cell.
            game.hoppers_catch_items(&original_moves);

            assert_eq!(game.dropped.len(), cases.len());
            let containers = game.containers();
            for (_, start, _) in cases {
                let view = containers.view((9, 64, start.z as i32), false).unwrap();
                assert!(containers.is_empty(&view));
            }
        });
    }

    fn check_adjacent_hopper_pickup(west_enabled: bool, previous_x: f64) {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let west = (8, 64, 8);
            let east = (9, 64, 8);
            let registry = &game.pack.assets.registry;
            let enabled = registry.default_state("minecraft:hopper").unwrap();
            let west_state = if west_enabled {
                enabled
            } else {
                let mut disabled = registry.state(enabled).unwrap().clone();
                disabled.properties.insert("enabled".into(), "false".into());
                registry.id(&disabled).unwrap()
            };
            let stone = registry.default_state("minecraft:stone").unwrap();
            for ((x, y, z), state) in [(west, west_state), (east, enabled)] {
                game.world.set(x, y, z, state).unwrap();
                game.world.set(x, y + 1, z, stone).unwrap();
                game.hoppers.insert((x, y, z), HopperState::default());
                game.world
                    .block_entities
                    .insert((x, y, z), BlockEntity::Chest(vec![None; HOPPER_SLOTS]));
            }
            // The item overlaps both pickup boxes and reaches into the west hopper's old bowl.
            let mut item = ItemEntity::new(
                2,
                ItemStack::new("minecraft:stone", 1),
                Vec3::new(8.95, 64.7, 8.5),
                Vec3::ZERO,
                0.0,
            );
            item.previous = Vec3::new(previous_x, 64.7, 8.5);
            let original_moves =
                FxHashMap::from_iter([(item.id, item.position.subtract(item.previous))]);
            game.dropped.push(item);
            game.hoppers_catch_items(&original_moves);

            assert!(game.dropped.is_empty());
            let containers = game.containers();
            let stayed_empty = containers.view(west, false).unwrap();
            assert!(containers.is_empty(&stayed_empty));
            let collected = containers.view(east, false).unwrap();
            let stack = containers.get(&collected, 0).unwrap();
            assert_eq!(stack.item.as_ref(), "minecraft:stone");
            assert_eq!(stack.count, 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn hopper_brewing_fuel_transfer_respects_stack_components() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let hopper_pos = (8, 64, 8);
            let stand_pos = (9, 64, 8);
            for (pos, block) in [
                (hopper_pos, "minecraft:hopper"),
                (stand_pos, "minecraft:brewing_stand"),
            ] {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
            }
            let default = ItemStack::new("minecraft:blaze_powder", 2);
            let mut custom = ItemStack::new("minecraft:stone", 2);
            custom.data_mut().set_loot_component(
                "minecraft:brewing_fuel",
                &Some(
                    horizon_core::json_nbt::json_to_tag(
                        &serde_json::json!({"uses": 7, "speed_multiplier": 2.5}),
                    )
                    .unwrap(),
                ),
            );
            let mut removed = default.clone();
            removed.data_mut().set_other(
                "!minecraft:brewing_fuel",
                Some(horizon_core::nbt::Tag::Compound(Default::default())),
            );
            for (stack, accepted) in [
                (default, true),
                (custom, true),
                (removed, false),
                (ItemStack::new("minecraft:stone", 2), false),
            ] {
                game.world
                    .block_entities
                    .insert(hopper_pos, BlockEntity::Chest(vec![Some(stack); 5]));
                game.world.block_entities.insert(
                    stand_pos,
                    BlockEntity::Brewing(Box::new(BrewingStand::new())),
                );
                let containers = game.containers();
                let hopper = containers.view(hopper_pos, false).unwrap();
                let stand = containers.view(stand_pos, false).unwrap();
                let fuel = containers.get(&hopper, 0).unwrap();
                assert_eq!(containers.can_place(&stand, 4, fuel), accepted);
                assert_eq!(
                    game.eject_into(hopper_pos, &hopper, &stand, Direction::West),
                    accepted
                );
                let containers = game.containers();
                assert_eq!(
                    containers.get(&hopper, 0).unwrap().count,
                    if accepted { 1 } else { 2 }
                );
                assert_eq!(
                    containers.get(&stand, 4).map(|s| s.count),
                    accepted.then_some(1)
                );
            }
        });
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
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            random: horizon_core::random::EntityRandom::own(
                horizon_core::random::LegacyRandom::new(0),
            ),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            health: 5,
            fire_ticks: 0,
            id,
            stack: ItemStack::new("minecraft:stone", 1),
            thrower: None,
            target: None,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: true,
            vertical_collision: false,
            horizontal_collision: false,
            age: 0,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
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

    /// An item moved in place (a `/tp`, an explosion's push) keeps the list's
    /// length and last id; the stamped columns still see where it went.
    #[test]
    fn a_moved_item_is_seen_where_it_went() {
        let mut items = horizon_client::join_seq::JoinList::<_, true>::from_iter([
            item(1, 0.5, 64.0, 0.5),
            item(2, 40.5, 64.0, 40.5),
        ]);
        let mut columns = super::StampedItemColumns::default();
        let far = ((100, 100), (100, 100));
        // The first question for a list scans; the second builds the columns.
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(!columns.may_reach_area(&items, far.0, far.1));
        items.update(0, |item| {
            item.position = horizon_core::geometry::Vec3::new(100.5, 64.0, 100.5)
        });
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(!columns.may_reach_area(&items, (0, 0), (0, 0)));
    }

    #[test]
    fn no_player_with_an_item_in_reach_is_skipped() {
        // guest_pickups' horizontal test, on every item.
        let touches = |items: &[ItemEntity], p: [f64; 3]| {
            items
                .iter()
                .any(|d| (d.position.x - p[0]).abs() < 1.5 && (d.position.z - p[2]).abs() < 1.5)
        };
        let mut seed = 0x0bad_5eed_u64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut items: Vec<ItemEntity> = Vec::new();
        let mut columns = ItemColumns::default();
        let (mut skipped, mut found) = (0, 0);
        for round in 0..4000u32 {
            if round % 25 == 0 {
                // On column edges as often as not.
                let mut at = || {
                    let v = next(40.0) - 20.0;
                    if next(1.0) < 0.5 { v.round() } else { v }
                };
                items.push(item(round, at(), 64.0, at()));
            } else if round % 101 == 0 && !items.is_empty() {
                items.remove(next(items.len() as f64) as usize);
            }
            let p = [next(44.0) - 22.0, 64.0, next(44.0) - 22.0];
            let reach = columns.may_reach_area(
                &items,
                ((p[0] - 1.5).floor() as i32, (p[0] + 1.5).floor() as i32),
                ((p[2] - 1.5).floor() as i32, (p[2] + 1.5).floor() as i32),
            );
            let touching = touches(&items, p);
            assert!(
                reach || !touching,
                "round {round}: skipped a player touching an item"
            );
            skipped += usize::from(!reach);
            found += usize::from(touching);
        }
        assert!(
            skipped > 1000 && found > 100,
            "skipped {skipped}, found {found}"
        );
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

#[cfg(test)]
mod mod_container_tests {
    use super::{Kind, ModContainer, View, room};
    use horizon_client::items::ItemStack;
    use horizon_core::geometry::Direction;
    use std::sync::Arc;

    /// A mod's container reaches the slots its block names for each face;
    /// without a face, every slot.
    #[test]
    fn mod_container_slots_by_face() {
        let mut faces: [Vec<usize>; 6] = Default::default();
        faces[Direction::Up as usize] = vec![0];
        faces[Direction::Down as usize] = vec![2];
        faces[Direction::North as usize] = vec![1];
        let view = View {
            parts: vec![((0, 0, 0), 3)],
            kind: Kind::Mod,
            container: Some(Arc::new(ModContainer {
                slots: 3,
                faces,
                inputs: vec![0, 1],
                outputs: vec![2],
                menu: None,
            })),
        };
        assert_eq!(
            view.slots_for_face_iter(Some(Direction::Up))
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            view.slots_for_face_iter(Some(Direction::Down))
                .collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(
            view.slots_for_face_iter(Some(Direction::North))
                .collect::<Vec<_>>(),
            [1]
        );
        assert!(
            view.slots_for_face_iter(Some(Direction::East))
                .next()
                .is_none()
        );
        assert_eq!(
            view.slots_for_face_iter(None).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    /// A slot takes no more than its container's limit and the item's: one book into a chiseled
    /// bookshelf slot of 64 offered, a chest slot 64 and then the rest of 64.
    #[test]
    fn a_slot_takes_up_to_its_containers_limit() {
        let books = |count| ItemStack {
            item: "minecraft:book".into(),
            count,
            damage: 0,
            extra: None,
        };
        let limit = |kind| {
            let view = View {
                parts: vec![((0, 0, 0), 6)],
                kind,
                container: None,
            };
            view.slot_limit().min(64)
        };
        assert_eq!(room(limit(Kind::Bookshelf), None, &books(64)), 1);
        assert_eq!(room(limit(Kind::Jukebox), None, &books(64)), 1);
        assert_eq!(room(limit(Kind::Generic), None, &books(64)), 64);
        assert_eq!(room(limit(Kind::Generic), Some(&books(60)), &books(64)), 4);
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::{Containers, Kind, View};
    use horizon_client::{brewing::Brewing, containers::BlockEntity, items::ItemStack};
    use std::collections::HashSet;

    /// A one-item slot answers its capacity as if empty (NeoForge's `getCapacityAsLong`), so a
    /// write may swap what it holds, while it takes nothing more now: a jukebox's disc, a
    /// chiseled bookshelf's book and a brewing stand's bottle.
    #[test]
    fn a_full_one_item_slot_still_has_room_for_a_replacement() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("block-behaviours.json").exists() {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = horizon_client::blocks::BlockFacts::load(&root, registry).unwrap();
        let overlay = &horizon_world::loot::PackOverlay::NONE;
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = horizon_client::items::Items::load(&root, registry, &facts, overlay, &defaults)
            .unwrap();
        let behaviours = horizon_world::level::Behaviours::load(registry, &root).unwrap();
        let brewing = Brewing {
            recipes: Vec::new(),
            reagents: HashSet::new(),
            inputs: HashSet::from(["minecraft:potion".to_owned()]),
        };
        let air = registry.default_state("minecraft:air").unwrap();
        let mut world = horizon_client::world::World::new(air, 0);
        let cases = [
            (
                "minecraft:jukebox",
                Kind::Jukebox,
                1,
                "minecraft:music_disc_cat",
                "minecraft:music_disc_13",
            ),
            (
                "minecraft:chiseled_bookshelf",
                Kind::Bookshelf,
                6,
                "minecraft:book",
                "minecraft:enchanted_book",
            ),
            (
                "minecraft:brewing_stand",
                Kind::Brewing,
                5,
                "minecraft:potion",
                "minecraft:potion",
            ),
        ];
        for (x, &(block, _, _, held, _)) in (0..).zip(&cases) {
            let mut entity = BlockEntity::for_block(block).unwrap();
            entity.slots_mut()[0] = Some(ItemStack::new(held, 1));
            world.block_entities.insert((x, 64, 0), entity);
        }
        let c = Containers {
            mobs: &[],
            world: &world,
            behaviours: &behaviours,
            items: &items,
            registry,
            brewing: Some(&brewing),
        };
        for (x, &(block, kind, len, _, offered)) in (0..).zip(&cases) {
            let view = View {
                parts: vec![((x, 64, 0), len)],
                kind,
                container: None,
            };
            let offered = ItemStack::new(offered, 1);
            assert_eq!(c.capacity(&view, 0, &offered), 1, "{block}");
            assert_eq!(c.fits(&view, 0, &offered), 0, "{block}");
            assert_eq!(
                c.capacity(&view, 0, &ItemStack::new("minecraft:stick", 1)),
                0,
                "{block}"
            );
        }
    }
}

#[cfg(test)]
mod part_front_tests {
    use super::super::native_entity_refs::tests::{declared_part_root, with_game_at};
    use horizon_client::chunk::Chunk;
    use horizon_client::containers::BlockEntity;
    use horizon_client::items::ItemStack;
    use horizon_core::position::BlockPos;
    use mod_api::BlockUpdateFlags;
    use std::sync::Arc;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    /// A crafter facing west into a pipe that hands on east, back into the crafter: the four
    /// sticks land in its grid, and using up the planks takes none of them.
    #[test]
    #[ignore = "daily"]
    fn a_part_handing_a_crafters_output_back_into_its_grid_loses_none() {
        let Some(root) = declared_part_root(
            "game::machines::part_front_tests::a_part_handing_a_crafters_output_back_into_its_grid_loses_none",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = horizon_world::chunk::BlockState {
                name: "minecraft:crafter".into(),
                properties: [("orientation".into(), "west_up".into())]
                    .into_iter()
                    .collect(),
            };
            let crafter = game.pack.assets.registry.resolve(&state).unwrap();
            assert_eq!(
                game.level_set_block(level, [2, 64, 8], crafter, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let pipe = game
                .pack
                .assets
                .registry
                .default_state("test:pipe")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 8], pipe, BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get_mut(&(2, 64, 8)) {
                c.items[3] = Some(stack("minecraft:oak_planks", 1));
                c.items[6] = Some(stack("minecraft:oak_planks", 1));
            }
            let dropped = game.dropped.len();
            game.dispense_from(BlockPos::new(2, 64, 8), crafter)
                .unwrap();
            let held = match game.world.block_entities.get(&(2, 64, 8)) {
                Some(BlockEntity::Crafter(c)) => c
                    .items
                    .iter()
                    .flatten()
                    .map(|s| (s.item.clone(), s.count))
                    .collect::<Vec<_>>(),
                _ => panic!("no crafter"),
            };
            assert_eq!(
                (held, game.dropped.len() - dropped),
                (vec![("minecraft:stick".into(), 4)], 0),
                "a pipe routing output back into the grid must conserve four crafted sticks"
            );
        });
    }

    /// A dropper facing west into a pipe that hands on east, back into the dropper: the item
    /// goes round, and taking it from its slot as it is then keeps both.
    #[test]
    #[ignore = "daily"]
    fn a_part_handing_a_droppers_item_back_into_it_loses_none() {
        let Some(root) = declared_part_root(
            "game::machines::part_front_tests::a_part_handing_a_droppers_item_back_into_it_loses_none",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = horizon_world::chunk::BlockState {
                name: "minecraft:dropper".into(),
                properties: [("facing".into(), "west".into())].into_iter().collect(),
            };
            let dropper = game.pack.assets.registry.resolve(&state).unwrap();
            assert_eq!(
                game.level_set_block(level, [2, 64, 8], dropper, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let pipe = game
                .pack
                .assets
                .registry
                .default_state("test:pipe")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 8], pipe, BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(2, 64, 8))
            {
                items[0] = Some(stack("minecraft:stick", 2));
            }
            let dropped = game.dropped.len();
            game.dispense_from(BlockPos::new(2, 64, 8), dropper)
                .unwrap();
            let held = match game.world.block_entities.get(&(2, 64, 8)) {
                Some(BlockEntity::Chest(items)) => {
                    items.iter().flatten().map(|s| s.count).sum::<u32>()
                }
                _ => panic!("no dropper"),
            };
            assert_eq!(
                (held, game.dropped.len() - dropped),
                (2, 0),
                "a pipe routing back into the source must conserve its items"
            );
        });
    }

    /// A dropper and a crafter facing a mod's item part (NeoForge's `dropperInsertHook` and
    /// `insertCrafterOutput`): a truthful part, a pipe into the chest east of it, takes what
    /// they put out in a container's place; a part answering more than it was offered has its
    /// move undone, so the dropper keeps its item and the crafter's result is dispensed.
    #[test]
    #[ignore = "daily"]
    fn a_dropper_and_a_crafter_put_into_a_mod_part_in_front_of_them() {
        let Some(root) = declared_part_root(
            "game::machines::part_front_tests::a_dropper_and_a_crafter_put_into_a_mod_part_in_front_of_them",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = |game: &super::Game<'_>, name: &str, prop: (&str, &str)| {
                let state = horizon_world::chunk::BlockState {
                    name: name.to_owned(),
                    properties: [(prop.0.to_owned(), prop.1.to_owned())]
                        .into_iter()
                        .collect(),
                };
                game.pack.assets.registry.resolve(&state).unwrap()
            };
            let dropper = state(game, "minecraft:dropper", ("facing", "east"));
            let crafter = state(game, "minecraft:crafter", ("orientation", "east_up"));
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            let chest_items =
                |game: &super::Game<'_>, z| match game.world.block_entities.get(&(2, 64, z)) {
                    Some(BlockEntity::Chest(items)) => {
                        items.iter().flatten().map(|s| s.count).sum::<u32>()
                    }
                    _ => panic!("no chest"),
                };
            for (z, part, taken) in [(0, "test:pipe", true), (4, "test:greedy", false)] {
                for (x, block) in [(0, dropper), (2, chest)] {
                    assert_eq!(
                        game.level_set_block(level, [x, 64, z], block, flags),
                        Ok(true)
                    );
                }
                let state = game.pack.assets.registry.default_state(part).unwrap();
                assert_eq!(
                    game.level_set_block(level, [1, 64, z], state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                if let Some(BlockEntity::Chest(items)) =
                    game.world.block_entities.get_mut(&(0, 64, z))
                {
                    items[4] = Some(stack("minecraft:stick", 2));
                }
                game.dispense_from(BlockPos::new(0, 64, z), dropper)
                    .unwrap();
                let left = match game.world.block_entities.get(&(0, 64, z)) {
                    Some(BlockEntity::Chest(items)) => items[4].as_ref().map_or(0, |s| s.count),
                    _ => panic!("no dropper"),
                };
                assert_eq!(
                    (left, chest_items(game, z)),
                    if taken { (1, 1) } else { (2, 0) },
                    "{part}"
                );

                // Two planks make four sticks.
                let z = z + 2;
                for (x, block) in [(0, crafter), (2, chest)] {
                    assert_eq!(
                        game.level_set_block(level, [x, 64, z], block, flags),
                        Ok(true)
                    );
                }
                let state = game.pack.assets.registry.default_state(part).unwrap();
                assert_eq!(
                    game.level_set_block(level, [1, 64, z], state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                if let Some(BlockEntity::Crafter(c)) =
                    game.world.block_entities.get_mut(&(0, 64, z))
                {
                    c.items[0] = Some(stack("minecraft:oak_planks", 1));
                    c.items[3] = Some(stack("minecraft:oak_planks", 1));
                }
                let items = game.dropped.len();
                game.dispense_from(BlockPos::new(0, 64, z), crafter)
                    .unwrap();
                let spawned = game.dropped.len() - items;
                assert_eq!(
                    (chest_items(game, z), spawned),
                    if taken { (4, 0) } else { (0, 1) },
                    "{part}"
                );
            }
        });
    }
}

#[cfg(test)]
#[path = "hopper_cart_rider_tests.rs"]
mod hopper_cart_rider_tests;

#[cfg(test)]
mod hopper_cart_transfer_tests {
    use super::super::{
        boats::Boat,
        minecarts::{CartKind, Minecart},
        native_entity_refs::tests::with_game,
    };
    use super::Game;
    use horizon_client::{containers::BlockEntity, items::ItemStack};
    use horizon_core::geometry::Vec3;
    use horizon_core::nbt::Tag;

    #[test]
    #[ignore = "daily"]
    fn chest_boat_automation_does_not_award_player_loot() {
        use horizon_client::advancements::Tree;
        use horizon_world::loot::PackOverlay;
        use std::sync::Arc;

        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = None;
            let files = vec![(
            "advancement/parity:boat_loot".into(),
            vec![br#"{"criteria":{"loot":{"trigger":"minecraft:player_generates_container_loot","conditions":{"loot_tables":"minecraft:chests/spawn_bonus_chest"}}}}"#.to_vec()],
        )];
            game.adv.tree = Arc::new(Tree::load_with(
                &game.options.resources.join("data"),
                &PackOverlay {
                    files: &files,
                    ..PackOverlay::NONE
                },
            ));
            game.adv.loaded = true;
            let ai = game.adv.tree.by_id["parity:boat_loot"];
            for player in [false, true] {
                game.next_entity += 1;
                let id = game.next_entity;
                game.boats
                    .push(Boat::new(id, "oak".into(), Vec3::new(8.5, 65.0, 8.5), 0.0).with_chest());
                game.persist.raw.insert(
                    id,
                    Tag::compound([
                        (
                            "LootTable",
                            Tag::String("minecraft:chests/spawn_bonus_chest".into()),
                        ),
                        ("LootTableSeed", Tag::Long(73)),
                    ]),
                );
                let i = game.boats.len() - 1;
                game.as_guest(12, |game| {
                    game.unpack_boat_loot(i, player);
                    assert_eq!(game.adv.progress.done(&game.adv.tree.list[ai]), player);
                    // Opening a container automation already unpacked awards nothing.
                    game.unpack_boat_loot(i, true);
                    assert_eq!(game.adv.progress.done(&game.adv.tree.list[ai]), player);
                })
                .unwrap();
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn chest_boat_with_items_drops_pending_table_without_rolling() {
        with_game(|game| {
            let sentinel = Some(ItemStack::new("minecraft:diamond", 17));
            for open in [false, true] {
                for occupied in [1, 24, 27] {
                    game.next_entity += 1;
                    let id = game.next_entity;
                    let mut boat =
                        Boat::new(id, "oak".into(), Vec3::new(8.5, 65.0, 8.5), 0.0).with_chest();
                    if occupied > 1 {
                        boat.items.fill(sentinel.clone());
                        if occupied == 24 {
                            for slot in [0, 13, 26] {
                                boat.items[slot] = None;
                            }
                        }
                    } else {
                        boat.items[12] = sentinel.clone();
                    }
                    let key = (
                        i32::MAX - 15,
                        super::super::minecarts::CART_KEY_Y,
                        id as i32,
                    );
                    if open {
                        game.world
                            .block_entities
                            .insert(key, BlockEntity::Chest(boat.items.clone()));
                        // The menu copy is authoritative, even if the entity is stale.
                        boat.items.fill(None);
                    }
                    game.boats.push(boat);
                    game.persist.raw.insert(
                        id,
                        Tag::compound([
                            (
                                "LootTable",
                                Tag::String("minecraft:chests/spawn_bonus_chest".into()),
                            ),
                            ("LootTableSeed", Tag::Long(73)),
                        ]),
                    );
                    let items = game
                        .with_entity_container(id, |g, _| {
                            g.world.block_entities.get(&key).unwrap().slots().to_vec()
                        })
                        .unwrap();
                    assert_eq!(items[12], sentinel);
                    // Horizon-only mixed state: never fill even the free slots.
                    assert_eq!(items.iter().flatten().count(), occupied);
                    assert!(
                        items
                            .iter()
                            .flatten()
                            .all(|item| Some(item) == sentinel.as_ref())
                    );
                    assert!(game.persist.raw[&id].get("LootTable").is_none());
                    assert!(game.persist.raw[&id].get("LootTableSeed").is_none());
                    assert_eq!(game.boats.last().unwrap().items, items);
                    game.with_entity_container(id, |g, _| {
                        if let Some(BlockEntity::Chest(items)) =
                            g.world.block_entities.get_mut(&key)
                        {
                            items.fill(None);
                        }
                    })
                    .unwrap();
                    game.with_entity_container(id, |g, _| {
                        assert!(
                            g.world
                                .block_entities
                                .get(&key)
                                .unwrap()
                                .slots()
                                .iter()
                                .all(Option::is_none)
                        );
                    })
                    .unwrap();
                    game.release_vehicle_container(id);
                }
            }
        });
    }

    /// Issue #8668: a hopper minecart takes one item a tick from the container
    /// above. `MinecartHopper.tick` has no transfer cooldown in 26.3: it calls
    /// `tryConsumeItems` every tick, and `consumedItemThisFrame` caps a tick at
    /// one item even though `makeStepAlongTrack` may try too. A whole stack a
    /// tick (the old behaviour) or a block hopper's eight-tick cooldown both
    /// fail this.
    #[test]
    #[ignore = "daily"]
    fn hopper_cart_takes_one_item_a_tick() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.streaming.forced.insert((0, 0));
            let rail = game
                .pack
                .assets
                .registry
                .default_state("minecraft:rail")
                .unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            // A rail holds the cart still; the chest sits in the block the cart
            // pulls from (`getLevelY` is y + 0.5, so the block above is y + 1.5).
            game.world.set(8, 64, 8, rail).unwrap();
            game.world.set(8, 65, 8, chest).unwrap();
            game.world.block_entities.insert(
                (8, 65, 8),
                BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:stone", 64)); 27]),
            );
            game.next_entity += 1;
            let id = game.next_entity;
            let mut cart = Minecart::new(id, CartKind::Hopper, Vec3::new(8.5, 64.0625, 8.5));
            cart.enabled = true;
            game.carts.push(cart);
            let count = |game: &Game<'_>| {
                let in_chest: u32 = match game.world.block_entities.get(&(8, 65, 8)) {
                    Some(BlockEntity::Chest(s)) => s.iter().flatten().map(|s| s.count).sum(),
                    _ => 0,
                };
                let in_cart: u32 = game
                    .carts
                    .iter()
                    .find(|c| c.id == id)
                    .unwrap()
                    .items
                    .iter()
                    .flatten()
                    .map(|s| s.count)
                    .sum();
                (in_chest, in_cart)
            };
            let (before_chest, before_cart) = count(game);
            for _ in 0..20 {
                game.tick_carts();
            }
            let (after_chest, after_cart) = count(game);
            assert_eq!(before_chest, 27 * 64);
            assert_eq!(before_cart, 0);
            assert_eq!(before_chest - after_chest, after_cart - before_cart);
            assert_eq!(
                after_cart, 20,
                "one item a tick, not a stack nor eight ticks"
            );
        });
    }
}

#[cfg(test)]
mod pot_hopper_insertion_tests {
    use super::*;
    use crate::game::{game_tests::tests::server_game, spill};
    use horizon_client::save;
    use horizon_core::nbt::Tag;

    fn count(game: &Game<'_>, pos: Pos, slot: usize) -> u32 {
        let Some(BlockEntity::Chest(slots)) = game.world.block_entities.get(&pos) else {
            panic!("actual command-created inventory missing at {pos:?}");
        };
        slots[slot].as_ref().map_or(0, |item| item.count)
    }
    fn pot(game: &Game<'_>, z: i32) -> (Tag, ItemStack) {
        let tag = spill::saved_entity(&game.world, [5, 200, z]).expect("actual saved pot");
        let item = save::item_from_nbt(tag.get("item").expect("positive actual item")).unwrap();
        (tag, item)
    }

    #[test]
    #[ignore = "daily"]
    fn actual_hopper_ticks_insert_into_the_saved_pot_slot() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        let mut initial = Vec::new();
        for (z, id, target) in [
            (4, "wheat", "decorated_pot"),
            (8, "carrot", "decorated_pot"),
            (12, "wheat", "chest"),
        ] {
            let command = format!(
                "setblock 4 200 {z} hopper[facing=east,enabled=true]{{Items:[{{Slot:0b,id:\"minecraft:{id}\",count:40,components:{{\"minecraft:custom_data\":{{h282:1}}}}}},{{Slot:4b,id:\"minecraft:diamond\",count:2}}],TransferCooldown:0}}"
            );
            assert!(game.run_command(&command));
            let content = if target == "decorated_pot" {
                "item:{id:\"minecraft:wheat\",count:3,components:{\"minecraft:custom_data\":{h282:1}}}"
            } else {
                "Items:[{Slot:0b,id:\"minecraft:wheat\",count:3,components:{\"minecraft:custom_data\":{h282:1}}}]"
            };
            assert!(game.run_command(&format!("setblock 5 200 {z} {target}{{{content}}}")));
            assert_eq!(count(&game, (4, 200, z), 0), 40);
            assert_eq!(count(&game, (4, 200, z), 4), 2);
            if target == "decorated_pot" {
                assert!(!game.world.block_entities.contains_key(&(5, 200, z)));
                assert!(
                    game.containers().view((5, 200, z), false).is_none(),
                    "generic discovery stays unchanged"
                );
                let (tag, item) = pot(&game, z);
                assert_eq!(item.count, 3);
                assert_eq!(&*item.item, "minecraft:wheat");
                initial.push((z, tag, item));
            }
        }
        assert_eq!(count(&game, (5, 200, 12), 0), 3);
        let start = game.world.game_time;
        let mut failures = Vec::new();
        for elapsed in 1..=217 {
            game.world.game_time = start + elapsed;
            for z in [4, 8, 12] {
                game.tick_hopper((4, 200, z));
            }
            if [1, 9, 217].contains(&elapsed) {
                let n = ((elapsed - 1) / 8 + 1) as u32;
                assert_eq!(
                    count(&game, (4, 200, 12), 0),
                    40 - n,
                    "ordinary hopper cadence"
                );
                assert_eq!(count(&game, (5, 200, 12), 0), 3 + n);
                assert_eq!(count(&game, (4, 200, 8), 0), 40, "different item rejected");
                assert_eq!(pot(&game, 8).1.count, 3);
                for z in [4, 8, 12] {
                    assert_eq!(count(&game, (4, 200, z), 4), 2);
                }
                let (mut saved, item) = pot(&game, 4);
                let mut before = initial[0].1.clone();
                if let (Tag::Compound(a), Tag::Compound(b)) = (&mut saved, &mut before) {
                    a.remove(&"item".into());
                    b.remove(&"item".into());
                }
                assert_eq!(saved, before, "raw metadata preserved");
                assert!(item.same_item_same_components(&initial[0].2));
                let donor = count(&game, (4, 200, 4), 0);
                assert_eq!(donor + item.count, 43, "actual conserved inventory");
                assert!(game.containers().view((5, 200, 4), false).is_none());
                if donor != 40 - n || item.count != 3 + n {
                    failures.push((elapsed, donor, item.count));
                }
            }
        }
        assert_eq!(game.world.game_time - start, 217);
        assert!(
            failures.is_empty(),
            "actual hopper/pot transfer missing: {failures:?}"
        );
    }
    #[test]
    #[ignore = "daily"]
    fn actual_hopper_pulls_from_the_saved_pot_slot() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(
            game.run_command(
                "setblock 5 201 4 decorated_pot{item:{id:\"minecraft:wheat\",count:2}}"
            )
        );
        assert!(
            game.run_command(
                "setblock 5 200 4 hopper[facing=down,enabled=true]{TransferCooldown:0}"
            )
        );
        let start = game.world.game_time;
        for elapsed in 1..=9 {
            game.world.game_time = start + elapsed;
            game.tick_hopper((5, 200, 4));
        }
        let Some(BlockEntity::Chest(slots)) = game.world.block_entities.get(&(5, 200, 4)) else {
            panic!("hopper inventory missing");
        };
        let held: u32 = slots.iter().flatten().map(|s| s.count).sum();
        assert_eq!(held, 2, "the hopper took both items");
        let tag = spill::saved_entity(&game.world, [5, 201, 4]).expect("saved pot");
        assert!(tag.get("item").is_none(), "the pot is empty");
    }
}

#[cfg(test)]
#[path = "machines_pot_signal_tests.rs"]
mod pot_signal_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::game_tests::tests::server_game;

    /// `ShelfBlockEntity.setChanged` (no arguments) posts `BLOCK_ACTIVATE`:
    /// a container writing a shelf's slot is a block activation.
    #[test]
    #[ignore = "daily"]
    fn a_container_writing_a_shelf_posts_block_activate() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let key = (2, 4, 3);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_shelf")
            .unwrap();
        game.world.set(key.0, key.1, key.2, state).unwrap();
        game.world
            .block_entities
            .insert(key, BlockEntity::Shelf(Box::default()));
        let view = game
            .containers()
            .view(key, false)
            .expect("the shelf's view");
        super::super::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));

        game.view_changed(&view);

        super::super::sculk::TEST_POSTED_EVENTS.with(|e| {
            assert!(
                e.borrow().as_ref().unwrap().contains(&(
                    "minecraft:block_activate",
                    [2.5, 4.5, 3.5],
                    None
                )),
                "{:?}",
                e.borrow()
            );
        });
    }
}

#[cfg(test)]
#[path = "machines_chest_cat_tests.rs"]
mod machines_chest_cat_tests;

#[cfg(test)]
mod can_take_tests {
    use super::*;
    use crate::game::game_tests::tests::server_game;
    use horizon_client::containers::ChiseledBookshelf;

    /// `ChiseledBookShelfBlockEntity.canTakeItem`: a hopper takes a book only
    /// when it has a free slot or a matching stack with room.
    #[test]
    #[ignore = "daily"]
    fn a_full_hopper_cannot_take_a_book_from_a_chiseled_shelf() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let shelf_pos = (2, 4, 3);
        let hopper_pos = (2, 5, 3);
        let shelf_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chiseled_bookshelf")
            .unwrap();
        let hopper_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:hopper")
            .unwrap();
        game.world
            .set(shelf_pos.0, shelf_pos.1, shelf_pos.2, shelf_state)
            .unwrap();
        game.world
            .set(hopper_pos.0, hopper_pos.1, hopper_pos.2, hopper_state)
            .unwrap();
        let book = ItemStack::new("minecraft:book", 1);
        let mut shelf = ChiseledBookshelf::default();
        shelf.items[0] = Some(book.clone());
        game.world
            .block_entities
            .insert(shelf_pos, BlockEntity::ChiseledBookshelf(Box::new(shelf)));
        let stone = ItemStack::new("minecraft:stone", 64);
        game.world
            .block_entities
            .insert(hopper_pos, BlockEntity::Chest(vec![Some(stone.clone()); 5]));
        let shelf_view = game.containers().view(shelf_pos, false).expect("shelf");
        let full_hopper = game.containers().view(hopper_pos, false).expect("hopper");
        assert!(
            !game
                .containers()
                .can_take_into(&shelf_view, 0, &book, Direction::Down, &full_hopper),
            "a full hopper takes no book"
        );
        game.world
            .block_entities
            .insert(hopper_pos, BlockEntity::Chest(vec![None; 5]));
        let empty_hopper = game.containers().view(hopper_pos, false).expect("hopper");
        assert!(
            game.containers()
                .can_take_into(&shelf_view, 0, &book, Direction::Down, &empty_hopper),
            "an empty hopper takes the book"
        );
    }

    /// A non-container block entity above a hopper (such as a sign holding lava)
    /// has no slots, so `view` returns `None`. The hopper does not treat it as a
    /// container, allowing item entities resting on the rim to be picked up.
    #[test]
    #[ignore = "daily"]
    fn non_container_block_entity_above_hopper_does_not_block_item_suction() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let hopper_pos = (2, 4, 3);
        let sign_pos = (2, 5, 3);
        let hopper_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:hopper")
            .unwrap();
        let sign_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_sign")
            .unwrap();
        game.world
            .set(hopper_pos.0, hopper_pos.1, hopper_pos.2, hopper_state)
            .unwrap();
        game.world
            .set(sign_pos.0, sign_pos.1, sign_pos.2, sign_state)
            .unwrap();
        game.world
            .block_entities
            .insert(hopper_pos, BlockEntity::Chest(vec![None; 5]));
        game.world
            .block_entities
            .insert(sign_pos, BlockEntity::Sign(Default::default()));

        assert!(
            game.containers().view(sign_pos, false).is_none(),
            "a sign is not a container"
        );
        assert!(
            game.containers().hopper_insertion_view(sign_pos).is_none(),
            "a sign has no hopper insertion view"
        );
        let hopper_view = game
            .containers()
            .view(hopper_pos, false)
            .expect("hopper view");
        assert!(
            game.suck_from_container(sign_pos, [2.5, 5.5, 3.5], &hopper_view)
                .is_none(),
            "suck_from_container returns None so hopper falls through to suck_item_entities"
        );
    }
}

#[cfg(test)]
mod stack_size_tests {
    use super::*;
    use crate::game::native_machines::tests::server_game;

    /// Stone with its own `max_stack_size`, or with the component removed
    /// (`!minecraft:max_stack_size`, vanilla's `getOrDefault` default of one).
    fn stone(size: Option<i32>, count: u32) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:stone", count);
        let data = stack.data_mut();
        match size {
            Some(n) => data.set_other("minecraft:max_stack_size", Some(Tag::Int(n))),
            None => data.set_other(
                "!minecraft:max_stack_size",
                Some(Tag::Compound(Default::default())),
            ),
        };
        stack.tidy();
        stack
    }

    /// `HopperBlockEntity.tryMoveInItem` fills a slot up to the incoming
    /// stack's `ItemStack.getMaxStackSize` (`itemStack.getMaxStackSize() -
    /// current.getCount()`), its own `max_stack_size`, not the item's default.
    #[test]
    fn a_hopper_stops_at_the_stacks_own_max_stack_size() {
        if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
            return;
        }
        let mut game = server_game();
        let chest = (2, 4, 3);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        game.world.set(chest.0, chest.1, chest.2, state).unwrap();
        // One slot, so what does not fit comes back as the remainder.
        let transfer = |game: &mut Game<'_>, slots: Vec<Option<ItemStack>>, stack: ItemStack| {
            game.world
                .block_entities
                .insert(chest, BlockEntity::Chest(slots));
            let view = game
                .containers()
                .view(chest, false)
                .expect("the chest's view");
            let rest = game.add_item(None, &view, stack, Some(Direction::Down));
            let count = game.containers().get(&view, 0).map(|s| s.count);
            (rest.map(|s| s.count), count)
        };
        // The slot takes four and holds two: two of the five go in.
        assert_eq!(
            transfer(&mut game, vec![Some(stone(Some(4), 2))], stone(Some(4), 5)),
            (Some(3), Some(4))
        );
        // A removed component holds one, so nothing merges into it.
        assert_eq!(
            transfer(&mut game, vec![Some(stone(None, 1))], stone(None, 2)),
            (Some(2), Some(1))
        );
    }
}
