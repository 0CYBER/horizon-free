//! `FlowingFluid` (water and lava) ticking and spreading, `LiquidBlock`
//! interactions, liquid containers (waterlogging) and bucket pickup.
use super::{
    Behaviours, LevelAccess, Sim, Spread,
    buckets::{Douse, Drain, PlacedLiquid},
    classes::{Class, Interface},
    update::{HORIZONTAL, above, below, rel},
};
use crate::{
    fluid::{FluidKind, FluidState, ModFluid, ModFluidRules},
    registry::BlockStateId,
};
use horizon_core::{
    geometry::{Axis, Direction, ShapeOp, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};

/// `LiquidBlock.POSSIBLE_FLOW_DIRECTIONS`.
const POSSIBLE_FLOW_DIRECTIONS: [Direction; 5] = [
    Direction::Down,
    Direction::South,
    Direction::North,
    Direction::East,
    Direction::West,
];

fn is_water(kind: FluidKind) -> bool {
    matches!(kind, FluidKind::Water | FluidKind::FlowingWater)
}
pub(super) fn source_of(kind: FluidKind) -> FluidKind {
    match kind {
        FluidKind::Mod(fluid) | FluidKind::ModFlowing(fluid) => FluidKind::Mod(fluid),
        _ if is_water(kind) => FluidKind::Water,
        _ => FluidKind::Lava,
    }
}
fn flowing_of(kind: FluidKind) -> FluidKind {
    match kind {
        FluidKind::Mod(fluid) | FluidKind::ModFlowing(fluid) => FluidKind::ModFlowing(fluid),
        _ if is_water(kind) => FluidKind::FlowingWater,
        _ => FluidKind::FlowingLava,
    }
}

/// `FlowingFluid.SpreadContext`: per-spread caches keyed by horizontal offset.
/// A spread looks at most `SPREAD_REACH` blocks away on each axis (the slope
/// search's 4 steps after the first), so the caches are flat arrays around
/// the origin: no hashing and no allocation on the hot path. A position
/// outside them is read uncached.
struct SpreadContext {
    origin: BlockPos,
    states: [Option<BlockStateId>; SPREAD_CELLS],
    holes: [Option<bool>; SPREAD_CELLS],
}
const SPREAD_REACH: i32 = 5;
const SPREAD_SIDE: i32 = 2 * SPREAD_REACH + 1;
const SPREAD_CELLS: usize = (SPREAD_SIDE * SPREAD_SIDE) as usize;
impl SpreadContext {
    fn new(origin: BlockPos) -> Self {
        Self {
            origin,
            states: [None; SPREAD_CELLS],
            holes: [None; SPREAD_CELLS],
        }
    }
    fn index(&self, pos: BlockPos) -> Option<usize> {
        let x = pos.x.wrapping_sub(self.origin.x) + SPREAD_REACH;
        let z = pos.z.wrapping_sub(self.origin.z) + SPREAD_REACH;
        ((0..SPREAD_SIDE).contains(&x) && (0..SPREAD_SIDE).contains(&z))
            .then(|| (x * SPREAD_SIDE + z) as usize)
    }
}

/// `Fluid.getTickDelay(level)` of `kind`: water 5, lava 30 (10 where lava flows fast, the Nether),
/// the empty fluid 0. `mod_rules` is asked only for a mod's fluid, for what it registered as its
/// parts changed it at the position.
pub fn fluid_tick_delay(
    kind: FluidKind,
    fast_lava: bool,
    mod_rules: impl FnOnce() -> Option<ModFluidRules>,
) -> i32 {
    match kind {
        FluidKind::Empty => 0,
        FluidKind::Water | FluidKind::FlowingWater => 5,
        FluidKind::Mod(fluid) | FluidKind::ModFlowing(fluid) => {
            mod_rules().unwrap_or(fluid.get().rules).tick_delay
        }
        FluidKind::Lava | FluidKind::FlowingLava => {
            if fast_lava {
                10
            } else {
                30
            }
        }
    }
}

/// `FluidState.canBeReplacedWith(level, pos, other, direction)`: whether `other`, flowing in
/// `direction`, may take the place of `fluid`. `height` is `fluid`'s height at the position, asked
/// only for lava, which water replaces from 4/9 of a block up.
pub fn fluid_can_be_replaced_with(
    fluid: FluidState,
    other: FluidKind,
    direction: Direction,
    height: impl FnOnce() -> f32,
) -> bool {
    match fluid.kind() {
        FluidKind::Empty => true,
        FluidKind::Water | FluidKind::FlowingWater => {
            direction == Direction::Down && !is_water(other)
        }
        // NeoForge's `BaseFlowingFluid.canBeReplacedWith`: nothing replaces it.
        FluidKind::Mod(_) | FluidKind::ModFlowing(_) => false,
        FluidKind::Lava | FluidKind::FlowingLava => height() >= 0.444_444_45 && is_water(other),
    }
}

/// `Fluid.getExplosionResistance` of `kind`: 100 for water, lava and a mod's fluid (NeoForge's
/// `BaseFlowingFluid` default), 0 for the empty fluid. Ray-cast explosions read the same 100 for
/// any block that holds a fluid.
pub fn fluid_explosion_resistance(kind: FluidKind) -> f32 {
    match kind {
        FluidKind::Empty => 0.0,
        _ => 100.0,
    }
}

/// `Fluid.isRandomlyTicking`: only lava's state ticks randomly (`FluidState.randomTick`).
pub fn fluid_randomly_ticks(kind: FluidKind) -> bool {
    matches!(kind, FluidKind::Lava | FluidKind::FlowingLava)
}

/// `Fluid.getDripParticle`: the particle a block of `kind` drips beneath it, which only water and
/// lava do (a mod's fluid and the empty fluid drip nothing).
pub fn fluid_drip_particle(kind: FluidKind) -> Option<&'static str> {
    match kind {
        FluidKind::Water | FluidKind::FlowingWater => Some("minecraft:dripping_water"),
        FluidKind::Lava | FluidKind::FlowingLava => Some("minecraft:dripping_lava"),
        FluidKind::Empty | FluidKind::Mod(_) | FluidKind::ModFlowing(_) => None,
    }
}

impl Behaviours {
    /// `state.getBlock() instanceof LiquidBlockContainer`: a block that keeps
    /// a fluid inside its own state (`SimpleWaterloggedBlock` extends the
    /// interface, so stairs, slabs and every other waterloggable block count).
    pub fn is_liquid_container(&self, state: BlockStateId) -> bool {
        self.implements(state, Interface::LiquidBlockContainer)
            || self.implements(state, Interface::SimpleWaterlogged)
    }
    /// `LiquidBlockContainer.canPlaceLiquid(user, level, pos, state, fluid)`
    /// for a liquid container: `creative` is whether `user` is a creative
    /// player (a bucket in creative mode); a dispenser or a flowing fluid
    /// passes none (`false`).
    pub fn can_place_liquid(&self, state: BlockStateId, fluid: FluidKind, creative: bool) -> bool {
        let data = self.data(state);
        if data.is(Class::Kelp)
            || data.is(Class::KelpPlant)
            || data.is(Class::Seagrass)
            || data.is(Class::TallSeagrass)
        {
            return false;
        }
        // `BarrierBlock.canPlaceLiquid`: only a creative player may waterlog.
        if data.is(Class::Barrier) && !creative {
            return false;
        }
        if data.is(Class::Slab) && self.table.text(state, self.props.kind) == "double" {
            return false;
        }
        fluid == FluidKind::Water
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn fast_lava(&self) -> bool {
        self.level.dimension_type().fast_lava
    }
    /// What a mod's fluid does at `pos`: as registered, unless its parts say otherwise.
    fn mod_rules(&self, fluid: ModFluid, pos: BlockPos) -> ModFluidRules {
        self.level.mod_fluid_rules(pos, fluid, fluid.get().rules)
    }
    /// `Fluid.getTickDelay`.
    pub(crate) fn fluid_tick_delay(&self, kind: FluidKind, pos: BlockPos) -> i32 {
        fluid_tick_delay(kind, self.fast_lava(), || {
            kind.mod_fluid().map(|fluid| self.mod_rules(fluid, pos))
        })
    }
    fn drop_off(&self, kind: FluidKind, pos: BlockPos) -> i32 {
        if let Some(fluid) = kind.mod_fluid() {
            return self.mod_rules(fluid, pos).drop_off;
        }
        if is_water(kind) || self.fast_lava() {
            1
        } else {
            2
        }
    }
    fn slope_find_distance(&self, kind: FluidKind, pos: BlockPos) -> i32 {
        if let Some(fluid) = kind.mod_fluid() {
            return self.mod_rules(fluid, pos).slope_find_distance;
        }
        if is_water(kind) || self.fast_lava() {
            4
        } else {
            2
        }
    }
    fn can_convert_to_source(&self, kind: FluidKind, pos: BlockPos) -> bool {
        if let Some(fluid) = kind.mod_fluid() {
            return self.mod_rules(fluid, pos).converts_to_source;
        }
        let rules = self.level.game_rules();
        if is_water(kind) {
            rules.water_source_conversion
        } else {
            rules.lava_source_conversion
        }
    }
    fn fluid_height(&self, fluid: FluidState, pos: BlockPos) -> f32 {
        if fluid.kind() == FluidKind::Empty {
            return 0.0;
        }
        if fluid.kind().same_family(self.fluid_at(above(pos)).kind()) {
            1.0
        } else {
            fluid.own_height()
        }
    }
    /// `FluidState.canBeReplacedWith(level, pos, other, direction)`.
    fn can_be_replaced_with(
        &self,
        fluid: FluidState,
        pos: BlockPos,
        other: FluidKind,
        direction: Direction,
    ) -> bool {
        fluid_can_be_replaced_with(fluid, other, direction, || self.fluid_height(fluid, pos))
    }

    // ---- shape occlusion ----

    fn collision_or_empty(&self, state: BlockStateId) -> (i32, &VoxelShape) {
        let index = self.b.facts(state).shape;
        match self.b.collision_shape(state) {
            Some(shape) => (index, shape),
            // `getCollisionShape(level, pos)` runs with `CollisionContext.empty()`,
            // under which scaffolding is above the block and not descending: it
            // reads as the stable shape. The other dynamic shapes stay empty.
            None if self.b.data(state).is(Class::Scaffolding) => {
                (-1, crate::dynamic_support::scaffolding_stable_shape_ref())
            }
            None => (-1, VoxelShape::empty_ref()),
        }
    }
    /// `FlowingFluid.canPassThroughWall`.
    fn can_pass_through_wall(
        &self,
        direction: Direction,
        source: BlockStateId,
        target: BlockStateId,
    ) -> bool {
        // The shapes are compared by identity and borrowed, never cloned: a
        // clone of the shared block or empty shape is atomic reference counts
        // every thread contends on, and this runs for every step of a spread.
        let (target_index, target_shape) = self.collision_or_empty(target);
        if target_shape.is_block() {
            return false;
        }
        let (source_index, source_shape) = self.collision_or_empty(source);
        if source_shape.is_block() {
            return false;
        }
        if source_shape.is_empty_identity() && target_shape.is_empty_identity() {
            return true;
        }
        let cacheable = source_index >= 0 && target_index >= 0;
        let key = (source_index, target_index, direction as u8);
        if cacheable
            && let Some(result) = self
                .b
                .occlusion_cache
                .read()
                .ok()
                .and_then(|cache| cache.get(&key).copied())
        {
            return result;
        }
        let result = !merged_face_occludes(source_shape, target_shape, direction);
        if cacheable && let Ok(mut cache) = self.b.occlusion_cache.write() {
            cache.insert(key, result);
        }
        result
    }

    // ---- liquid containers ----

    /// `LiquidBlockContainer` membership (`canHoldAnyFluid` first clause).
    pub(super) fn is_liquid_container(&self, state: BlockStateId) -> bool {
        self.b.is_liquid_container(state)
    }
    /// `LiquidBlockContainer.canPlaceLiquid` (`Behaviours::can_place_liquid`).
    pub(crate) fn can_place_liquid(
        &self,
        state: BlockStateId,
        fluid: FluidKind,
        creative: bool,
    ) -> bool {
        self.b.can_place_liquid(state, fluid, creative)
    }
    /// `LiquidBlockContainer.placeLiquid`.
    pub(crate) fn place_liquid(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        fluid: FluidState,
        creative: bool,
    ) -> bool {
        let Some(placed) = self.plan_place_liquid(pos, state, fluid, creative) else {
            return false;
        };
        self.apply_placed_liquid(pos, placed);
        true
    }
    /// What `placeLiquid` does once decided: put a lit block out, set the waterlogged block,
    /// tick the water and play a dried ghast's sound.
    pub(crate) fn apply_placed_liquid(&mut self, pos: BlockPos, placed: PlacedLiquid) {
        self.douse(pos, placed.douse);
        self.set_block_and_update(pos, placed.to);
        self.schedule_fluid_tick(pos, placed.fluid, placed.delay);
        self.placed_in_water(pos, placed);
    }
    /// What `LiquidBlockContainer.placeLiquid` of `fluid` into `state` at `pos` does, decided
    /// against the level as it is: None where it refuses (another fluid than water, a block
    /// without `waterlogged` or one waterlogged already, or `canPlaceLiquid` saying no).
    pub(crate) fn plan_place_liquid(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        fluid: FluidState,
        creative: bool,
    ) -> Option<PlacedLiquid> {
        let b = self.b;
        if !self.can_place_liquid(state, fluid.kind(), creative)
            || fluid.kind() != FluidKind::Water
            || !b.table.has(state, b.props.waterlogged)
            || b.is_waterlogged(state)
        {
            return None;
        }
        let mut to = b.table.set_bool(state, b.props.waterlogged, true);
        let data = b.data(state);
        let douse = if (data.is(Class::Candle) || data.is(Class::Campfire))
            && b.table.bool(state, b.props.lit)
        {
            to = b.table.set_bool(to, b.props.lit, false);
            Some(match data.is(Class::Campfire) {
                true => Douse::Campfire,
                false => Douse::Candle,
            })
        } else {
            None
        };
        Some(PlacedLiquid {
            to,
            fluid: fluid.kind(),
            delay: self.fluid_tick_delay(fluid.kind(), pos),
            douse,
            dried_ghast: data.is(Class::DriedGhast),
        })
    }
    /// `CampfireBlock.douse` and `AbstractCandleBlock.extinguish`: the sound, then
    /// `level.gameEvent(null, GameEvent.BLOCK_CHANGE, pos)`, both before `placeLiquid` sets the
    /// waterlogged block.
    pub(crate) fn douse(&mut self, pos: BlockPos, douse: Option<Douse>) {
        let Some(douse) = douse else {
            return;
        };
        let sound = match douse {
            Douse::Campfire => "minecraft:entity.generic.extinguish_fire",
            Douse::Candle => "minecraft:block.candle.extinguish",
        };
        self.emit(super::LevelEvent::Sound {
            pos,
            sound: sound.into(),
            volume: 1.0,
            pitch: 1.0,
        });
        self.game_event(pos, "minecraft:block_change");
    }
    /// `DriedGhastBlock.placeLiquid`: its own place-in-water sound, after the block is set.
    pub(crate) fn placed_in_water(&mut self, pos: BlockPos, placed: PlacedLiquid) {
        if placed.dried_ghast {
            self.emit(super::LevelEvent::Sound {
                pos,
                sound: "minecraft:block.dried_ghast.place_in_water".into(),
                volume: 1.0,
                pitch: 1.0,
            });
        }
    }
    /// `BucketPickup.pickupBlock`: true when a fluid was taken.
    pub(crate) fn pickup_block(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        creative: bool,
    ) -> bool {
        let Some(drain) = self.plan_pickup_block(pos, state, creative) else {
            return false;
        };
        self.set_block(pos, drain.to, drain.flags);
        if drain.burst {
            // PowderSnowBlock.pickupBlock: the block breaks with its particles and sound.
            self.level_event(2001, pos, state.0 as i32);
        }
        // Asked after the write, as vanilla asks it.
        if drain.drained && !self.can_survive(state, pos) {
            self.destroy_block(pos, true, 512);
        }
        true
    }
    /// What `BucketPickup.pickupBlock` does to `state` at `pos`, decided against the level as
    /// it is: None where the block gives nothing.
    pub(crate) fn plan_pickup_block(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        creative: bool,
    ) -> Option<Drain> {
        let b = self.b;
        let removed = |burst| Drain {
            to: b.air(),
            flags: 11,
            drained: false,
            breaks: false,
            burst,
        };
        if b.instance_of(state, Class::Liquid) {
            return (b.table.int(state, b.props.level) == 0).then(|| removed(false));
        }
        if b.implements(state, Interface::SimpleWaterlogged)
            && b.table.has(state, b.props.waterlogged)
        {
            // `BarrierBlock.pickupBlock`: only a creative player drains one.
            if (b.instance_of(state, Class::Barrier) && !creative) || !b.is_waterlogged(state) {
                return None;
            }
            // `SimpleWaterloggedBlock.pickupBlock`: a block that can't survive drained breaks.
            return Some(Drain {
                to: b.table.set_bool(state, b.props.waterlogged, false),
                flags: super::flags::UPDATE_ALL,
                drained: true,
                breaks: !self.can_survive(state, pos),
                burst: false,
            });
        }
        if b.is_block(state, b.named.bubble_column) {
            // BubbleColumnBlock.pickupBlock: the column is removed, the bucket is water.
            return Some(removed(false));
        }
        b.is_block(state, b.named.powder_snow)
            .then(|| removed(true))
    }
    fn can_hold_any_fluid(&self, state: BlockStateId) -> bool {
        self.is_liquid_container(state) || self.b.in_set(state, &self.b.tags.washed_away_by_fluids)
    }
    fn can_hold_specific_fluid(&self, state: BlockStateId, fluid: FluidKind) -> bool {
        if self.is_liquid_container(state) {
            self.can_place_liquid(state, fluid, false)
        } else {
            true
        }
    }
    fn can_hold_fluid(&self, state: BlockStateId, fluid: FluidKind) -> bool {
        self.can_hold_any_fluid(state) && self.can_hold_specific_fluid(state, fluid)
    }

    // ---- FlowingFluid ----

    fn is_source_of(&self, kind: FluidKind, fluid: FluidState) -> bool {
        fluid.kind().same_family(kind) && fluid.is_source()
    }
    /// `FlowingFluid.getNewLiquid`.
    fn new_liquid(&self, kind: FluidKind, pos: BlockPos, state: BlockStateId) -> FluidState {
        let b = self.b;
        let mut highest = 0;
        let mut sources = 0;
        let converts = self.can_convert_to_source(kind, pos);
        for direction in HORIZONTAL {
            let neighbour = self.rel_state(pos, direction);
            let fluid = b.fluid(neighbour);
            if fluid.kind().same_family(kind)
                && self.can_pass_through_wall(direction, state, neighbour)
            {
                // Mods are asked for each neighbouring source, the fluid's rule
                // as the default (`CreateFluidSourceEvent`).
                if fluid.is_source()
                    && self
                        .level
                        .create_fluid_source(rel(pos, direction), neighbour, converts)
                {
                    sources += 1;
                }
                highest = highest.max(i32::from(fluid.amount()));
            }
        }
        if sources >= 2 {
            let below_state = self.below_state(pos);
            if b.is_solid(below_state) || self.is_source_of(kind, b.fluid(below_state)) {
                return FluidState::new(source_of(kind), 8, false);
            }
        }
        let above_state = self.above_state(pos);
        let above_fluid = b.fluid(above_state);
        if above_fluid.kind() != FluidKind::Empty
            && above_fluid.kind().same_family(kind)
            && self.can_pass_through_wall(Direction::Up, state, above_state)
        {
            return FluidState::new(flowing_of(kind), 8, true);
        }
        let amount = highest - self.drop_off(kind, pos);
        if amount <= 0 {
            FluidState::EMPTY
        } else {
            FluidState::new(flowing_of(kind), amount as u8, false)
        }
    }
    fn is_water_hole(&self, kind: FluidKind, top: BlockStateId, bottom: BlockStateId) -> bool {
        if !self.can_pass_through_wall(Direction::Down, top, bottom) {
            return false;
        }
        if self.b.fluid(bottom).kind().same_family(kind) {
            true
        } else {
            self.can_hold_fluid(bottom, flowing_of(kind))
        }
    }
    fn can_maybe_pass_through(
        &self,
        kind: FluidKind,
        source: BlockStateId,
        direction: Direction,
        test: BlockStateId,
        test_fluid: FluidState,
    ) -> bool {
        !self.is_source_of(kind, test_fluid)
            && self.can_hold_any_fluid(test)
            && self.can_pass_through_wall(direction, source, test)
    }
    fn can_pass_through(
        &self,
        kind: FluidKind,
        source: BlockStateId,
        direction: Direction,
        test: BlockStateId,
        test_fluid: FluidState,
    ) -> bool {
        self.can_maybe_pass_through(kind, source, direction, test, test_fluid)
            && self.can_hold_specific_fluid(test, flowing_of(kind))
    }
    fn context_state(&self, context: &mut SpreadContext, pos: BlockPos) -> BlockStateId {
        let Some(i) = context.index(pos) else {
            return self.state(pos);
        };
        if let Some(state) = context.states[i] {
            return state;
        }
        let state = self.state(pos);
        context.states[i] = Some(state);
        state
    }
    fn context_is_hole(&self, kind: FluidKind, context: &mut SpreadContext, pos: BlockPos) -> bool {
        let index = context.index(pos);
        if let Some(hole) = index.and_then(|i| context.holes[i]) {
            return hole;
        }
        let state = self.context_state(context, pos);
        let below_state = self.below_state(pos);
        let hole = self.is_water_hole(kind, state, below_state);
        if let Some(i) = index {
            context.holes[i] = Some(hole);
        }
        hole
    }
    fn slope_distance(
        &self,
        kind: FluidKind,
        pos: BlockPos,
        pass: i32,
        from: Direction,
        state: BlockStateId,
        context: &mut SpreadContext,
    ) -> i32 {
        let mut lowest = 1000;
        for direction in HORIZONTAL {
            if direction == from {
                continue;
            }
            let test_pos = rel(pos, direction);
            let test_state = self.context_state(context, test_pos);
            let test_fluid = self.b.fluid(test_state);
            if self.can_pass_through(kind, state, direction, test_state, test_fluid) {
                if self.context_is_hole(kind, context, test_pos) {
                    return pass;
                }
                if pass < self.slope_find_distance(kind, test_pos) {
                    let distance = self.slope_distance(
                        kind,
                        test_pos,
                        pass + 1,
                        direction.opposite(),
                        test_state,
                        context,
                    );
                    lowest = lowest.min(distance);
                }
            }
        }
        lowest
    }
    /// `FlowingFluid.getSpread` (EnumMap: iteration in Direction order).
    fn spread_targets(
        &self,
        kind: FluidKind,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Vec<(Direction, FluidState)> {
        let mut lowest = 1000;
        let mut result: Vec<(Direction, FluidState)> = Vec::new();
        let mut context: Option<SpreadContext> = None;
        for direction in HORIZONTAL {
            let test_pos = rel(pos, direction);
            let test_state = self.state(test_pos);
            let test_fluid = self.b.fluid(test_state);
            if !self.can_maybe_pass_through(kind, state, direction, test_state, test_fluid) {
                continue;
            }
            let new_fluid = self.new_liquid(kind, test_pos, test_state);
            if !self.can_hold_specific_fluid(test_state, new_fluid.kind()) {
                continue;
            }
            let context = context.get_or_insert_with(|| SpreadContext::new(pos));
            let distance = if self.context_is_hole(kind, context, test_pos) {
                0
            } else {
                self.slope_distance(kind, test_pos, 1, direction.opposite(), test_state, context)
            };
            if distance < lowest {
                result.clear();
            }
            if distance <= lowest {
                if self.can_be_replaced_with(test_fluid, test_pos, new_fluid.kind(), direction) {
                    result.push((direction, new_fluid));
                }
                lowest = distance;
            }
        }
        result.sort_by_key(|(direction, _)| *direction as u8);
        result
    }
    fn source_neighbour_count(&self, kind: FluidKind, pos: BlockPos) -> i32 {
        HORIZONTAL
            .into_iter()
            .filter(|d| self.is_source_of(kind, self.fluid_at(rel(pos, *d))))
            .count() as i32
    }
    /// `FlowingFluid.spreadTo` (with LavaFluid's water-to-stone override).
    fn spread_to(
        &mut self,
        kind: FluidKind,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        target: FluidState,
    ) {
        let b = self.b;
        let from = rel(pos, direction.opposite());
        if !self.level.allow_spread(Spread::Fluid, from, pos) {
            return;
        }
        let lava = kind.same_family(FluidKind::Lava);
        if lava && direction == Direction::Down && b.is_water_fluid(self.fluid_at(pos).kind()) {
            if b.instance_of(state, Class::Liquid)
                && let Some(state) =
                    self.level
                        .fluid_place_block(pos, from, b.default_of(b.named.stone))
            {
                self.set_block_and_update(pos, state);
            }
            self.level_event(1501, pos, 0);
            return;
        }
        if self.is_liquid_container(state) {
            self.place_liquid(pos, state, target, false);
        } else {
            if !b.is_air(state) {
                // Only `LavaFluid.beforeDestroyingBlock` hisses instead of dropping.
                if lava {
                    self.level_event(1501, pos, 0);
                } else {
                    self.drop_resources(state, pos);
                }
            }
            let legacy = b.legacy_block(target);
            self.set_block_and_update(pos, legacy);
        }
    }
    fn spread_to_sides(
        &mut self,
        kind: FluidKind,
        pos: BlockPos,
        fluid: FluidState,
        state: BlockStateId,
    ) {
        let mut neighbour = i32::from(fluid.amount()) - self.drop_off(kind, pos);
        if fluid.falling() {
            neighbour = 7;
        }
        if neighbour > 0 {
            for (direction, new_fluid) in self.spread_targets(kind, pos, state) {
                let neighbour_pos = rel(pos, direction);
                let neighbour_state = self.state(neighbour_pos);
                self.spread_to(kind, neighbour_pos, neighbour_state, direction, new_fluid);
            }
        }
    }
    /// `FlowingFluid.spread`.
    fn spread(&mut self, kind: FluidKind, pos: BlockPos, state: BlockStateId, fluid: FluidState) {
        if fluid.kind() == FluidKind::Empty {
            return;
        }
        let below_pos = below(pos);
        let below_state = self.state(below_pos);
        let below_fluid = self.b.fluid(below_state);
        if self.can_maybe_pass_through(kind, state, Direction::Down, below_state, below_fluid) {
            let new_below = self.new_liquid(kind, below_pos, below_state);
            if self.can_be_replaced_with(below_fluid, below_pos, new_below.kind(), Direction::Down)
                && self.can_hold_specific_fluid(below_state, new_below.kind())
            {
                self.spread_to(kind, below_pos, below_state, Direction::Down, new_below);
                if self.source_neighbour_count(kind, pos) >= 3 {
                    self.spread_to_sides(kind, pos, fluid, state);
                }
                return;
            }
        }
        if fluid.is_source() || !self.is_water_hole(kind, state, below_state) {
            self.spread_to_sides(kind, pos, fluid, state);
        }
    }
    /// `FlowingFluid.tick` for the fluid at `pos`.
    pub(crate) fn fluid_tick(&mut self, pos: BlockPos) {
        let mut state = self.state(pos);
        let mut fluid = self.b.fluid(state);
        let kind = fluid.kind();
        let name = match kind {
            FluidKind::Water | FluidKind::FlowingWater => "water",
            FluidKind::Lava | FluidKind::FlowingLava => "lava",
            FluidKind::Mod(_) | FluidKind::ModFlowing(_) => kind.name(),
            FluidKind::Empty => return,
        };
        if !self.level.allow_fluid_tick(pos, name) {
            return;
        }
        if !fluid.is_source() {
            let new_fluid = self.new_liquid(kind, pos, state);
            let delay = self.spread_delay(kind, pos, fluid, new_fluid);
            if new_fluid != fluid
                && !self.level.allow_fluid_level_change(
                    pos,
                    name,
                    i32::from(fluid.amount()),
                    i32::from(new_fluid.amount()),
                )
            {
                return;
            }
            if new_fluid.kind() == FluidKind::Empty {
                fluid = new_fluid;
                state = self.b.air();
                self.set_block_and_update(pos, state);
            } else if new_fluid != fluid {
                fluid = new_fluid;
                state = self.b.legacy_block(fluid);
                self.set_block_and_update(pos, state);
                self.schedule_fluid_tick(pos, fluid.kind(), delay);
            }
        }
        self.spread(kind, pos, state, fluid);
    }
    fn spread_delay(
        &mut self,
        kind: FluidKind,
        pos: BlockPos,
        old: FluidState,
        new: FluidState,
    ) -> i32 {
        let delay = self.fluid_tick_delay(kind, pos);
        // Only `LavaFluid.getSpreadDelay` differs from `Fluid.getTickDelay`.
        if !kind.same_family(FluidKind::Lava) {
            return delay;
        }
        if old.kind() != FluidKind::Empty
            && new.kind() != FluidKind::Empty
            && !old.falling()
            && !new.falling()
            && self.fluid_height(new, pos) > self.fluid_height(old, pos)
            && self.random().next_int_bound(4).unwrap_or(0) != 0
        {
            delay * 4
        } else {
            delay
        }
    }

    // ---- LiquidBlock ----

    /// `LiquidBlock.shouldSpreadLiquid`, including lava's conversions.
    pub(crate) fn should_spread_liquid(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        let b = self.b;
        if !b.is_lava_fluid(b.fluid(state).kind()) {
            return true;
        }
        let over_soul_soil = b.is_block(self.below_state(pos), b.named.soul_soil);
        for direction in POSSIBLE_FLOW_DIRECTIONS {
            let neighbour = rel(pos, direction.opposite());
            if b.is_water_fluid(self.fluid_at(neighbour).kind()) {
                let block = if self.fluid_at(pos).is_source() {
                    b.named.obsidian
                } else {
                    b.named.cobblestone
                };
                if let Some(state) = self.level.fluid_place_block(pos, pos, b.default_of(block)) {
                    self.set_block_and_update(pos, state);
                }
                self.level_event(1501, pos, 0);
                return false;
            }
            if over_soul_soil && b.is_block(self.state(neighbour), b.named.blue_ice) {
                if let Some(state) =
                    self.level
                        .fluid_place_block(pos, pos, b.default_of(b.named.basalt))
                {
                    self.set_block_and_update(pos, state);
                }
                self.level_event(1501, pos, 0);
                return false;
            }
        }
        true
    }
    /// `LiquidBlock.onPlace` / `neighborChanged`.
    pub(crate) fn liquid_block_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        if self.should_spread_liquid(pos, state) {
            let kind = self.b.fluid(state).kind();
            let delay = self.fluid_tick_delay(kind, pos);
            self.schedule_fluid_tick(pos, kind, delay);
        }
        if self.should_bubble_column_occupy(state) {
            let below_state = self.below_state(pos);
            let block = self.b.block(state);
            self.try_schedule_bubble_column(pos, below_state, block);
        }
    }

    /// `LavaFluid.randomTick`.
    pub(crate) fn lava_random_tick(&mut self, pos: BlockPos) {
        if !self.level.can_spread_fire_around(pos) {
            return;
        }
        let passes = self.random().next_int_bound(3).unwrap_or(0);
        if passes > 0 {
            let mut test = pos;
            for _ in 0..passes {
                let dx = self.random().next_int_bound(3).unwrap_or(0) - 1;
                let dz = self.random().next_int_bound(3).unwrap_or(0) - 1;
                test = test.offset(dx, 1, dz);
                if !self.level.is_loaded(test) {
                    return;
                }
                let state = self.state(test);
                if self.b.is_air(state) {
                    if Direction::ALL
                        .into_iter()
                        .any(|d| self.lava_flammable(rel(test, d), d.opposite()))
                    {
                        if !self.level.allow_block_ignite(test, "lava") {
                            return;
                        }
                        let fire = self.fire_state_for(test);
                        if let Some(state) = self.level.fluid_place_block(test, pos, fire) {
                            self.set_block_and_update(test, state);
                        }
                        return;
                    }
                } else if self
                    .b
                    .tag("minecraft:blocks_lava_fire_spread")
                    .is_some_and(|tag| self.b.in_set(state, tag))
                {
                    return;
                }
            }
        } else {
            for _ in 0..3 {
                let dx = self.random().next_int_bound(3).unwrap_or(0) - 1;
                let dz = self.random().next_int_bound(3).unwrap_or(0) - 1;
                let test = pos.offset(dx, 0, dz);
                if !self.level.is_loaded(test) {
                    return;
                }
                if self.is_empty_block(above(test)) && self.lava_flammable(test, Direction::Up) {
                    if !self.level.allow_block_ignite(above(test), "lava") {
                        continue;
                    }
                    let fire = self.fire_state_for(test);
                    if let Some(state) = self.level.fluid_place_block(above(test), pos, fire) {
                        self.set_block_and_update(above(test), state);
                    }
                }
            }
        }
    }
    /// `LavaFluid.isFlammable` of the block at `pos`, asked from its side
    /// `face`: the level's answer, else the state's `ignitedByLava` flag.
    fn lava_flammable(&self, pos: BlockPos, face: Direction) -> bool {
        if self.is_inside_build_height(pos.y) && !self.level.is_loaded(pos) {
            return false;
        }
        let state = self.state(pos);
        let mut ignites = None;
        self.level
            .before_ignited_by_lava(state, pos, face, &mut ignites);
        ignites.unwrap_or_else(|| {
            self.b
                .has_flag(state, super::data::state_flags::IGNITED_BY_LAVA)
        })
    }
}

/// `Shapes.mergedFaceOccludes(shape, occluder, direction)`.
pub(crate) fn merged_face_occludes(
    shape: &VoxelShape,
    occluder: &VoxelShape,
    direction: Direction,
) -> bool {
    let block = VoxelShape::block();
    if shape.is_block() || occluder.is_block() {
        return true;
    }
    let positive = matches!(
        direction,
        Direction::Up | Direction::South | Direction::East
    );
    let axis = match direction {
        Direction::West | Direction::East => Axis::X,
        Direction::Down | Direction::Up => Axis::Y,
        _ => Axis::Z,
    };
    let (first, second) = if positive {
        (shape, occluder)
    } else {
        (occluder, shape)
    };
    let max_face = match axis {
        Axis::X => Direction::East,
        Axis::Y => Direction::Up,
        Axis::Z => Direction::South,
    };
    let empty = VoxelShape::empty();
    // SliceShape(first, axis, last layer) / SliceShape(second, axis, 0): the face
    // shapes sample the same boundary layers when the shapes touch the face.
    let first_slice = if !first.is_empty() && (first.max(axis) - 1.0).abs() <= 1.0e-7 {
        first.face_shape(max_face).unwrap_or_else(|_| empty.clone())
    } else {
        empty.clone()
    };
    let second_slice = if !second.is_empty() && second.min(axis).abs() <= 1.0e-7 {
        second
            .face_shape(max_face.opposite())
            .unwrap_or_else(|_| empty.clone())
    } else {
        empty.clone()
    };
    let union = match first_slice.join(&second_slice, ShapeOp::OR) {
        Ok(shape) => shape,
        Err(_) => return false,
    };
    !block
        .join_is_not_empty(&union, ShapeOp::ONLY_FIRST)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluid::{ModFluidType, register_mod_fluid};

    fn mod_fluid(id: &str, tick_delay: i32) -> FluidKind {
        FluidKind::Mod(
            register_mod_fluid(ModFluidType {
                id: format!("fluid_facts:{id}"),
                flowing_id: format!("fluid_facts:flowing_{id}"),
                bucket: None,
                rules: ModFluidRules {
                    tick_delay,
                    drop_off: 1,
                    slope_find_distance: 4,
                    converts_to_source: false,
                },
            })
            .unwrap(),
        )
    }

    /// Vanilla's `getTickDelay`: water 5 in every dimension, lava 30 and 10 where it flows fast
    /// (the Nether), the empty fluid nothing; a flowing form delays as its source.
    #[test]
    fn tick_delay_follows_the_fluid_and_the_dimension() {
        let delay = |kind, fast| fluid_tick_delay(kind, fast, || None);
        for fast in [false, true] {
            assert_eq!(delay(FluidKind::Water, fast), 5);
            assert_eq!(delay(FluidKind::FlowingWater, fast), 5);
            assert_eq!(delay(FluidKind::Empty, fast), 0);
        }
        assert_eq!(delay(FluidKind::Lava, false), 30);
        assert_eq!(delay(FluidKind::FlowingLava, false), 30);
        assert_eq!(delay(FluidKind::Lava, true), 10);
        assert_eq!(delay(FluidKind::FlowingLava, true), 10);
    }

    /// A mod's fluid takes the delay it registered, or the one its parts answer, in any
    /// dimension; the rules are never asked for a vanilla fluid.
    #[test]
    fn a_mod_fluid_delays_as_registered_or_as_its_parts_say() {
        let oil = mod_fluid("tick_delay", 20);
        for fast in [false, true] {
            assert_eq!(fluid_tick_delay(oil, fast, || None), 20);
        }
        let faster = ModFluidRules {
            tick_delay: 7,
            ..oil.mod_fluid().unwrap().get().rules
        };
        assert_eq!(fluid_tick_delay(oil, false, || Some(faster)), 7);
        assert_eq!(
            fluid_tick_delay(FluidKind::Lava, false, || unreachable!()),
            30
        );
    }

    /// Water gives way only to a non-water fluid flowing down; lava to water once it stands 4/9
    /// of a block high; the empty fluid to anything; a mod's fluid to nothing.
    #[test]
    fn replacement_follows_vanilla_for_water_lava_empty_and_mod_fluids() {
        let flow = |fluid: FluidState, other, direction, height: f32| {
            fluid_can_be_replaced_with(fluid, other, direction, || height)
        };
        let water = FluidState::WATER;
        let lava = FluidState::new(FluidKind::Lava, 8, false);
        for direction in Direction::ALL {
            assert!(flow(FluidState::EMPTY, FluidKind::Water, direction, 0.0));
            assert_eq!(
                flow(water, FluidKind::Lava, direction, 1.0),
                direction == Direction::Down
            );
            assert!(!flow(water, FluidKind::FlowingWater, direction, 1.0));
            assert!(!flow(lava, FluidKind::Lava, direction, 1.0));
            assert!(!flow(
                FluidState::new(mod_fluid("replace", 5), 8, false),
                FluidKind::Water,
                direction,
                1.0
            ));
        }
        let from_the_side = |height| flow(lava, FluidKind::FlowingWater, Direction::East, height);
        assert!(from_the_side(8.0 / 9.0));
        assert!(from_the_side(4.0 / 9.0));
        assert!(!from_the_side(3.0 / 9.0));
        assert!(!flow(lava, FluidKind::Lava, Direction::East, 8.0 / 9.0));
    }

    /// Vanilla's fluid constants: 100 for a fluid, nothing for the empty one; only lava ticks
    /// randomly; only water and lava drip, as their own particles.
    #[test]
    fn fluid_constants_match_vanilla() {
        let oil = mod_fluid("constants", 5);
        let flowing_oil = FluidKind::ModFlowing(oil.mod_fluid().unwrap());
        for (kind, resistance, random, drip) in [
            (FluidKind::Empty, 0.0, false, None),
            (
                FluidKind::Water,
                100.0,
                false,
                Some("minecraft:dripping_water"),
            ),
            (
                FluidKind::FlowingWater,
                100.0,
                false,
                Some("minecraft:dripping_water"),
            ),
            (
                FluidKind::Lava,
                100.0,
                true,
                Some("minecraft:dripping_lava"),
            ),
            (
                FluidKind::FlowingLava,
                100.0,
                true,
                Some("minecraft:dripping_lava"),
            ),
            (oil, 100.0, false, None),
            (flowing_oil, 100.0, false, None),
        ] {
            assert_eq!(fluid_explosion_resistance(kind), resistance, "{kind:?}");
            assert_eq!(fluid_randomly_ticks(kind), random, "{kind:?}");
            assert_eq!(fluid_drip_particle(kind), drip, "{kind:?}");
        }
    }
}
