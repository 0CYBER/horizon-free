//! Tree decorators (TreeDecorator subclasses), in native random-draw order.
use super::{
    Layer, NestedPlacer, Run, TreeDecoder, TreeResult, kind, read, sample_optional, sample_state,
    unit_float,
};
use crate::heightmap::HeightmapType;
use crate::native_worldgen_hooks::{
    self, AlterGroundTree, GroundAccess, ModGroundProvider, WorldgenRandom,
};
use crate::registry::BlockStateId;
use crate::worldgen::{
    DecodeError, Object,
    feature::FeatureWorld,
    feature_graph::{FeatureDefinition, FeatureExecutionError},
    int_range, optional,
    placement::PlacementContext,
    required,
    state_provider::BlockStateProvider,
    structure::processor::ProcessorRandom,
    tree_blocks::{self as tb, TreeBlocks},
};
use horizon_core::{geometry::Direction, random::RandomSource};
use serde_json::value::RawValue;
use std::collections::BTreeSet;

/// Catalog-injected configured feature for the pale-moss ground patch, which
/// vanilla looks up by registry key during decoration.
pub(crate) const GROUND_FEATURE_KEY: &str = "horizon:pale_moss_patch";

/// Decorator context lists: set iteration order, stably sorted by Y.
pub(crate) struct Lists {
    pub logs: Vec<[i32; 3]>,
    pub leaves: Vec<[i32; 3]>,
    pub roots: Vec<[i32; 3]>,
}
impl Lists {
    pub(crate) fn new(
        mut logs: Vec<[i32; 3]>,
        mut leaves: Vec<[i32; 3]>,
        mut roots: Vec<[i32; 3]>,
    ) -> Self {
        logs.sort_by_key(|p| p[1]);
        leaves.sort_by_key(|p| p[1]);
        roots.sort_by_key(|p| p[1]);
        Self {
            logs,
            leaves,
            roots,
        }
    }
    /// TreeFeature.getLowestTrunkOrRootOfTree.
    fn lowest(&self) -> Vec<[i32; 3]> {
        if self.roots.is_empty() {
            self.logs.clone()
        } else if !self.logs.is_empty() && self.roots[0][1] == self.logs[0][1] {
            let mut all = self.logs.clone();
            all.extend(&self.roots);
            all
        } else {
            self.roots.clone()
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum TreeDecorator {
    TrunkVine,
    LeaveVine(f32),
    Cocoa(f32),
    Beehive(f32),
    AlterGround(BlockStateProvider),
    AttachedToLeaves {
        probability: f32,
        exclusion_xz: i32,
        exclusion_y: i32,
        provider: BlockStateProvider,
        required_empty: i32,
        directions: Vec<Direction>,
    },
    AttachedToLogs {
        probability: f32,
        provider: BlockStateProvider,
        directions: Vec<Direction>,
    },
    CreakingHeart(f32),
    PaleMoss {
        leaves: f32,
        trunk: f32,
        ground: f32,
        patch: Option<Box<FeatureDefinition>>,
    },
    PlaceOnGround {
        tries: i32,
        radius: i32,
        height: i32,
        provider: BlockStateProvider,
    },
    ShelfMushroom(f32),
}

fn directions(raw: &RawValue) -> Result<Vec<Direction>, DecodeError> {
    let names: Vec<String> = serde_json::from_str(raw.get())?;
    if names.is_empty() || names.len() > 64 {
        return Err(DecodeError::Field("decorator directions"));
    }
    names.iter().map(|n| tb::decode_direction(n)).collect()
}

pub(crate) type NestedDecoder<'a> =
    dyn Fn(&str, usize, &mut usize) -> Result<FeatureDefinition, DecodeError> + 'a;

impl TreeDecorator {
    pub(crate) fn decode_list(
        raw: &RawValue,
        decoder: &mut TreeDecoder<'_, '_>,
        ground: Option<&RawValue>,
        nested: &NestedDecoder<'_>,
    ) -> Result<Vec<Self>, DecodeError> {
        let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
        if values.len() > 256 {
            return Err(DecodeError::Field("tree decorator count"));
        }
        values
            .iter()
            .map(|raw| Self::decode(raw, decoder, ground, nested))
            .collect()
    }
    fn decode(
        raw: &RawValue,
        decoder: &mut TreeDecoder<'_, '_>,
        ground: Option<&RawValue>,
        nested: &NestedDecoder<'_>,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(raw.get())?;
        let probability = || unit_float(required(&object, "probability")?, "probability");
        Ok(match kind(&object)?.as_str() {
            "minecraft:trunk_vine" => Self::TrunkVine,
            "minecraft:leave_vine" => Self::LeaveVine(probability()?),
            "minecraft:cocoa" => Self::Cocoa(probability()?),
            "minecraft:beehive" => Self::Beehive(probability()?),
            "minecraft:creaking_heart" => Self::CreakingHeart(probability()?),
            "minecraft:shelf_mushroom" => Self::ShelfMushroom(probability()?),
            "minecraft:alter_ground" => {
                Self::AlterGround(decoder.provider(required(&object, "provider")?)?)
            }
            "minecraft:attached_to_leaves" => Self::AttachedToLeaves {
                probability: probability()?,
                exclusion_xz: int_range(
                    required(&object, "exclusion_radius_xz")?,
                    "exclusion_radius_xz",
                    0,
                    16,
                )?,
                exclusion_y: int_range(
                    required(&object, "exclusion_radius_y")?,
                    "exclusion_radius_y",
                    0,
                    16,
                )?,
                provider: decoder.provider(required(&object, "block_provider")?)?,
                required_empty: int_range(
                    required(&object, "required_empty_blocks")?,
                    "required_empty_blocks",
                    1,
                    16,
                )?,
                directions: directions(required(&object, "directions")?)?,
            },
            "minecraft:attached_to_logs" => Self::AttachedToLogs {
                probability: probability()?,
                provider: decoder.provider(required(&object, "block_provider")?)?,
                directions: directions(required(&object, "directions")?)?,
            },
            "minecraft:pale_moss" => Self::PaleMoss {
                leaves: unit_float(
                    required(&object, "leaves_probability")?,
                    "leaves_probability",
                )?,
                trunk: unit_float(required(&object, "trunk_probability")?, "trunk_probability")?,
                ground: unit_float(
                    required(&object, "ground_probability")?,
                    "ground_probability",
                )?,
                patch: ground
                    .map(|raw| nested(raw.get(), decoder.depth + 1, decoder.remaining))
                    .transpose()?
                    .map(Box::new),
            },
            "minecraft:place_on_ground" => Self::PlaceOnGround {
                tries: optional(&object, "tries")
                    .map(|raw| int_range(raw, "tries", 1, i32::MAX))
                    .transpose()?
                    .unwrap_or(128),
                radius: optional(&object, "radius")
                    .map(|raw| int_range(raw, "radius", 0, i32::MAX))
                    .transpose()?
                    .unwrap_or(2),
                height: optional(&object, "height")
                    .map(|raw| int_range(raw, "height", 0, i32::MAX))
                    .transpose()?
                    .unwrap_or(1),
                provider: decoder.provider(required(&object, "block_state_provider")?)?,
            },
            _ => return Err(DecodeError::Field("tree decorator type")),
        })
    }

    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        run: &mut Run<'_, W, R>,
        blocks: &TreeBlocks,
        lists: &Lists,
        nested: &mut NestedPlacer<'_, W, R>,
    ) -> TreeResult<(), W::Error> {
        let mut d = Decorate { run, blocks };
        match self {
            Self::TrunkVine => {
                for &pos in &lists.logs {
                    for (direction, face) in [
                        (Direction::West, "east"),
                        (Direction::East, "west"),
                        (Direction::North, "south"),
                        (Direction::South, "north"),
                    ] {
                        if d.run.random.next_int_bound(3)? > 0 {
                            let p = tb::relative(pos, direction, 1);
                            if d.is_air(p)? {
                                d.place_vine(p, face)?;
                            }
                        }
                    }
                }
            }
            Self::LeaveVine(probability) => {
                for &pos in &lists.leaves {
                    for (direction, face) in [
                        (Direction::West, "east"),
                        (Direction::East, "west"),
                        (Direction::North, "south"),
                        (Direction::South, "north"),
                    ] {
                        if d.run.random.next_float() < *probability {
                            let p = tb::relative(pos, direction, 1);
                            if d.is_air(p)? {
                                d.place_vine(p, face)?;
                                let mut below = tb::offset(p, 0, -1, 0);
                                let mut remaining = 4;
                                while d.is_air(below)? && remaining > 0 {
                                    d.place_vine(below, face)?;
                                    below = tb::offset(below, 0, -1, 0);
                                    remaining -= 1;
                                }
                            }
                        }
                    }
                }
            }
            Self::Cocoa(probability) => {
                if d.run.random.next_float() >= *probability {
                    return Ok(());
                }
                let Some(first) = lists.logs.first() else {
                    return Ok(());
                };
                let tree_y = first[1];
                for &pos in lists.logs.iter().filter(|p| p[1].wrapping_sub(tree_y) <= 2) {
                    for direction in tb::HORIZONTAL {
                        if d.run.random.next_float() <= 0.25 {
                            let n = direction.opposite().normal();
                            let p = tb::offset(pos, n[0], 0, n[2]);
                            if d.is_air(p)? {
                                let age = d.run.random.next_int_bound(3)?;
                                let state = d.default("minecraft:cocoa")?;
                                let state = d.with(state, "age", &age.to_string())?;
                                let state =
                                    d.with(state, "facing", tb::direction_name(direction))?;
                                d.set(p, state)?;
                            }
                        }
                    }
                }
            }
            Self::Beehive(probability) => d.beehive(lists, *probability)?,
            Self::AlterGround(provider) => {
                let positions = lists.lowest();
                if positions.is_empty() {
                    return Ok(());
                }
                let replaced = native_worldgen_hooks::alter_ground(&AlterGroundTree {
                    logs: &lists.logs,
                    leaves: &lists.leaves,
                    roots: &lists.roots,
                    positions: &positions,
                });
                let ground = Ground {
                    decorator: provider,
                    replaced: replaced.as_deref(),
                };
                d.alter_ground(&ground, &positions)?;
            }
            Self::AttachedToLeaves {
                probability,
                exclusion_xz,
                exclusion_y,
                provider,
                required_empty,
                directions,
            } => {
                let mut blacklist = BTreeSet::new();
                let mut leaves = lists.leaves.clone();
                tb::shuffle(&mut leaves, d.run.random)?;
                for leaf in leaves {
                    let direction =
                        directions[d.run.random.next_int_bound(directions.len() as i32)? as usize];
                    let p = tb::relative(leaf, direction, 1);
                    if blacklist.contains(&p) || d.run.random.next_float() >= *probability {
                        continue;
                    }
                    let mut empty = true;
                    for i in 1..=*required_empty {
                        if !d.is_air(tb::relative(leaf, direction, i))? {
                            empty = false;
                            break;
                        }
                    }
                    if !empty {
                        continue;
                    }
                    for dx in -exclusion_xz..=*exclusion_xz {
                        for dy in -exclusion_y..=*exclusion_y {
                            for dz in -exclusion_xz..=*exclusion_xz {
                                blacklist.insert(tb::offset(p, dx, dy, dz));
                            }
                        }
                    }
                    let state = sample_state(provider, d.run.world, d.run.random, p)?;
                    d.set(p, state)?;
                }
            }
            Self::AttachedToLogs {
                probability,
                provider,
                directions,
            } => {
                let mut logs = lists.logs.clone();
                tb::shuffle(&mut logs, d.run.random)?;
                for log in logs {
                    let direction =
                        directions[d.run.random.next_int_bound(directions.len() as i32)? as usize];
                    let p = tb::relative(log, direction, 1);
                    if d.run.random.next_float() <= *probability && d.is_air(p)? {
                        let state = sample_state(provider, d.run.world, d.run.random, p)?;
                        d.set(p, state)?;
                    }
                }
            }
            Self::CreakingHeart(probability) => {
                if lists.logs.is_empty() || d.run.random.next_float() >= *probability {
                    return Ok(());
                }
                let mut candidates = lists.logs.clone();
                tb::shuffle(&mut candidates, d.run.random)?;
                for pos in candidates {
                    let mut enclosed = true;
                    for direction in Direction::ALL {
                        let state = read(d.run.world, tb::relative(pos, direction, 1))?;
                        if !blocks.has(state, tb::LOGS) {
                            enclosed = false;
                            break;
                        }
                    }
                    if enclosed {
                        let state = d.default("minecraft:creaking_heart")?;
                        let state = d.with(state, "creaking_heart_state", "dormant")?;
                        let state = d.with(state, "natural", "true")?;
                        d.set(pos, state)?;
                        break;
                    }
                }
            }
            Self::PaleMoss {
                leaves,
                trunk,
                ground,
                patch,
            } => {
                let mut logs = lists.logs.clone();
                tb::shuffle(&mut logs, d.run.random)?;
                let Some(origin) = logs
                    .iter()
                    .copied()
                    .reduce(|best, p| if p[1] < best[1] { p } else { best })
                else {
                    return Ok(());
                };
                if d.run.random.next_float() < *ground
                    && let Some(patch) = patch
                {
                    nested(
                        patch,
                        d.run.world,
                        d.run.random,
                        tb::offset(origin, 0, 1, 0),
                    )?;
                }
                for (positions, chance) in [(&lists.logs, *trunk), (&lists.leaves, *leaves)] {
                    for &pos in positions {
                        if d.run.random.next_float() < chance {
                            let down = tb::offset(pos, 0, -1, 0);
                            if d.is_air(down)? {
                                d.moss_hanger(down)?;
                            }
                        }
                    }
                }
            }
            Self::PlaceOnGround {
                tries,
                radius,
                height,
                provider,
            } => {
                let positions = lists.lowest();
                let Some(&origin) = positions.first() else {
                    return Ok(());
                };
                let min_y = origin[1];
                let (mut min_x, mut max_x, mut min_z, mut max_z) =
                    (origin[0], origin[0], origin[2], origin[2]);
                for p in positions.iter().filter(|p| p[1] == min_y) {
                    min_x = min_x.min(p[0]);
                    max_x = max_x.max(p[0]);
                    min_z = min_z.min(p[2]);
                    max_z = max_z.max(p[2]);
                }
                let low = [
                    min_x.wrapping_sub(*radius),
                    min_y.wrapping_sub(*height),
                    min_z.wrapping_sub(*radius),
                ];
                let high = [
                    max_x.wrapping_add(*radius),
                    min_y.wrapping_add(*height),
                    max_z.wrapping_add(*radius),
                ];
                for _ in 0..*tries {
                    let p = [
                        d.run.random.next_int_between_inclusive(low[0], high[0])?,
                        d.run.random.next_int_between_inclusive(low[1], high[1])?,
                        d.run.random.next_int_between_inclusive(low[2], high[2])?,
                    ];
                    let above = tb::offset(p, 0, 1, 0);
                    let above_state = read(d.run.world, above)?;
                    if !blocks.has(above_state, tb::AIR | tb::VINE) {
                        continue;
                    }
                    if !blocks.has(read(d.run.world, p)?, tb::SOLID_RENDER) {
                        continue;
                    }
                    let surface = d
                        .run
                        .world
                        .height(HeightmapType::MotionBlockingNoLeaves, p[0], p[2])
                        .map_err(FeatureExecutionError::Query)?;
                    if surface <= above[1] {
                        let state = sample_state(provider, d.run.world, d.run.random, above)?;
                        d.set(above, state)?;
                    }
                }
            }
            Self::ShelfMushroom(probability) => d.shelf_mushroom(lists, *probability)?,
        }
        Ok(())
    }
}

/// The provider `AlterGroundDecorator` places ground with: its own, or the one
/// mods put in its place (`AlterGroundEvent.getStateProvider`).
struct Ground<'p> {
    decorator: &'p BlockStateProvider,
    replaced: Option<&'p dyn ModGroundProvider>,
}

/// The decoration run as a mod's ground provider reads it. A failed read is
/// kept and returned once the provider answers.
struct GroundRun<'a, 'w, W: FeatureWorld, R> {
    run: &'a mut Run<'w, W, R>,
    decorator: &'a BlockStateProvider,
    air: BlockStateId,
    error: Option<FeatureExecutionError<W::Error>>,
}
impl<W: FeatureWorld + PlacementContext, R: RandomSource> GroundAccess for GroundRun<'_, '_, W, R> {
    fn block_state(&mut self, pos: [i32; 3]) -> BlockStateId {
        match read(self.run.world, pos) {
            Ok(state) => state,
            Err(error) => {
                self.error.get_or_insert(error);
                self.air
            }
        }
    }
    fn decorator_state(&mut self, pos: [i32; 3]) -> Option<BlockStateId> {
        match sample_optional(self.decorator, self.run.world, self.run.random, pos) {
            Ok(state) => state,
            Err(error) => {
                self.error.get_or_insert(error);
                None
            }
        }
    }
}
impl<W: FeatureWorld, R: RandomSource> WorldgenRandom for GroundRun<'_, '_, W, R> {
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        ProcessorRandom::int(self.run.random, bound)
    }
    fn next_long(&mut self) -> i64 {
        self.run.random.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.run.random.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.run.random.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.run.random.next_boolean()
    }
}

struct Decorate<'a, 'b, 'w, W, R> {
    run: &'a mut Run<'w, W, R>,
    blocks: &'b TreeBlocks,
}
impl<W: FeatureWorld + PlacementContext, R: RandomSource> Decorate<'_, '_, '_, W, R> {
    fn is_air(&mut self, p: [i32; 3]) -> TreeResult<bool, W::Error> {
        Ok(self.blocks.is_air(read(self.run.world, p)?))
    }
    fn has(&mut self, p: [i32; 3], flag: u32) -> TreeResult<bool, W::Error> {
        Ok(self.blocks.has(read(self.run.world, p)?, flag))
    }
    fn set(&mut self, p: [i32; 3], state: BlockStateId) -> TreeResult<(), W::Error> {
        self.run.set(Layer::Decoration, p, state)
    }
    fn default(&self, name: &str) -> TreeResult<BlockStateId, W::Error> {
        self.blocks
            .default_state(name)
            .ok_or(FeatureExecutionError::InvalidState)
    }
    fn with(
        &self,
        state: BlockStateId,
        name: &str,
        value: &str,
    ) -> TreeResult<BlockStateId, W::Error> {
        self.blocks
            .with(state, name, value)
            .ok_or(FeatureExecutionError::InvalidState)
    }
    fn place_vine(&mut self, p: [i32; 3], face: &str) -> TreeResult<(), W::Error> {
        let vine = self.default("minecraft:vine")?;
        let state = self.with(vine, face, "true")?;
        self.set(p, state)
    }
    /// `AlterGroundDecorator.place` around `positions`, never empty.
    fn alter_ground(
        &mut self,
        ground: &Ground<'_>,
        positions: &[[i32; 3]],
    ) -> TreeResult<(), W::Error> {
        let min_y = positions[0][1];
        for &pos in positions.iter().filter(|p| p[1] == min_y) {
            self.circle(ground, tb::offset(pos, -1, 0, -1))?;
            self.circle(ground, tb::offset(pos, 2, 0, -1))?;
            self.circle(ground, tb::offset(pos, -1, 0, 2))?;
            self.circle(ground, tb::offset(pos, 2, 0, 2))?;
            for _ in 0..5 {
                let placement = self.run.random.next_int_bound(64)?;
                let (xx, zz) = (placement % 8, placement / 8);
                if xx == 0 || xx == 7 || zz == 0 || zz == 7 {
                    self.circle(ground, tb::offset(pos, -3 + xx, 0, -3 + zz))?;
                }
            }
        }
        Ok(())
    }
    /// The ground state at `cursor`: the decorator's own provider, or the
    /// mods' in its place. A state the registry doesn't have is refused.
    fn ground_state(
        &mut self,
        ground: &Ground<'_>,
        cursor: [i32; 3],
    ) -> TreeResult<Option<BlockStateId>, W::Error> {
        let Some(replaced) = ground.replaced else {
            return sample_optional(ground.decorator, self.run.world, self.run.random, cursor);
        };
        let mut access = GroundRun {
            air: self.default("minecraft:air")?,
            run: &mut *self.run,
            decorator: ground.decorator,
            error: None,
        };
        let state = replaced.state_at(&mut access, cursor);
        if let Some(error) = access.error {
            return Err(error);
        }
        match state {
            Some(state) if !self.blocks.is_state(state) => Err(FeatureExecutionError::InvalidState),
            state => Ok(state),
        }
    }
    fn circle(&mut self, ground: &Ground<'_>, center: [i32; 3]) -> TreeResult<(), W::Error> {
        for xx in -2i32..=2 {
            for zz in -2i32..=2 {
                if xx.abs() != 2 || zz.abs() != 2 {
                    let column = tb::offset(center, xx, 0, zz);
                    for dy in (-3..=2).rev() {
                        let cursor = tb::offset(column, 0, dy, 0);
                        if let Some(state) = self.ground_state(ground, cursor)? {
                            self.set(cursor, state)?;
                            break;
                        }
                        if !self.is_air(cursor)? && dy < 0 {
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn moss_hanger(&mut self, mut p: [i32; 3]) -> TreeResult<(), W::Error> {
        let moss = self.default("minecraft:pale_hanging_moss")?;
        let middle = self.with(moss, "tip", "false")?;
        let tip = self.with(moss, "tip", "true")?;
        while self.is_air(tb::offset(p, 0, -1, 0))? && self.run.random.next_float() >= 0.5 {
            self.set(p, middle)?;
            p = tb::offset(p, 0, -1, 0);
        }
        self.set(p, tip)
    }
    fn beehive(&mut self, lists: &Lists, probability: f32) -> TreeResult<(), W::Error> {
        let (Some(first_log), Some(last_log)) = (lists.logs.first(), lists.logs.last()) else {
            return Ok(());
        };
        if self.run.random.next_float() >= probability {
            return Ok(());
        }
        let hive_y = match lists.leaves.first() {
            Some(leaf) => (leaf[1] - 1).max(first_log[1] + 1),
            None => (first_log[1] + 1 + self.run.random.next_int_bound(3)?).min(last_log[1]),
        };
        let mut placements: Vec<[i32; 3]> = lists
            .logs
            .iter()
            .filter(|p| p[1] == hive_y)
            .flat_map(|p| {
                [Direction::East, Direction::South, Direction::West].map(|d| tb::relative(*p, d, 1))
            })
            .collect();
        if placements.is_empty() {
            return Ok(());
        }
        tb::shuffle(&mut placements, self.run.random)?;
        for p in placements {
            if self.is_air(p)? && self.is_air(tb::relative(p, Direction::South, 1))? {
                let nest = self.default("minecraft:bee_nest")?;
                let nest = self.with(nest, "facing", "south")?;
                self.set(p, nest)?;
                // Occupants exist only when the write produced the nest block entity.
                if read(self.run.world, p)? == nest {
                    let count = 2 + self.run.random.next_int_bound(2)?;
                    let mut ticks = Vec::with_capacity(count as usize);
                    for _ in 0..count {
                        ticks.push(self.run.random.next_int_bound(599)?);
                    }
                    self.run
                        .world
                        .store_beehive_occupants(p, &ticks)
                        .map_err(FeatureExecutionError::Query)?;
                }
                break;
            }
        }
        Ok(())
    }
    fn shelf_mushroom(&mut self, lists: &Lists, probability: f32) -> TreeResult<(), W::Error> {
        if self.run.random.next_float() >= probability {
            return Ok(());
        }
        let (Some(first), Some(last)) = (lists.logs.first(), lists.logs.last()) else {
            return Ok(());
        };
        if first[1] == last[1] {
            let facings = if first[0] != last[0] {
                [Direction::North, Direction::South]
            } else {
                [Direction::East, Direction::West]
            };
            for &log in &lists.logs {
                for facing in facings {
                    if self.run.random.next_float() <= 0.25 {
                        let p = mushroom_pos(log, facing);
                        if self.mushroom_replaceable(p)?
                            && !self.adjacent_mushroom(p)?
                            && !self.adjacent_mushroom(log)?
                        {
                            self.place_mushroom(p, facing)?;
                        }
                    }
                }
            }
        } else {
            let a = tb::HORIZONTAL[self.run.random.next_int_bound(4)? as usize];
            let facings = [a, tb::clockwise(a)];
            let base = first[1];
            for &log in &lists.logs {
                let dy = log[1].wrapping_sub(base);
                if !(1..=4).contains(&dy) {
                    continue;
                }
                for facing in facings {
                    if self.run.random.next_float() <= 0.25 {
                        let p = mushroom_pos(log, facing);
                        if self.mushroom_replaceable(p)?
                            && !self.has(tb::offset(p, 0, -1, 0), tb::SHELF_MUSHROOM)?
                        {
                            self.place_mushroom(p, facing)?;
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn mushroom_replaceable(&mut self, p: [i32; 3]) -> TreeResult<bool, W::Error> {
        if !self.has(p, tb::REPLACEABLE)? {
            return Ok(false);
        }
        for q in [
            p,
            tb::offset(p, 1, 0, 0),
            tb::offset(p, -1, 0, 0),
            tb::offset(p, 0, 0, -1),
            tb::offset(p, 0, 0, 1),
        ] {
            if self.has(q, tb::WATER_BLOCK)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn adjacent_mushroom(&mut self, p: [i32; 3]) -> TreeResult<bool, W::Error> {
        for direction in tb::HORIZONTAL {
            if self.has(tb::relative(p, direction, 1), tb::SHELF_MUSHROOM)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn place_mushroom(&mut self, p: [i32; 3], facing: Direction) -> TreeResult<(), W::Error> {
        let age = self.run.random.next_int_bound(2)?;
        let state = self.default("minecraft:shelf_mushroom")?;
        let state = self.with(state, "age", &age.to_string())?;
        let state = self.with(state, "facing", tb::direction_name(facing))?;
        self.set(p, state)
    }
}
fn mushroom_pos(log: [i32; 3], facing: Direction) -> [i32; 3] {
    let n = facing.normal();
    tb::offset(log, n[0], 0, n[2])
}

/// The nest's block entity after `BeehiveDecorator` stored its bees
/// (`Occupant.create(ticks)`: a bee, with 600 minimum ticks in the hive).
pub(crate) fn beehive_occupants(ticks_in_hive: &[i32]) -> horizon_core::nbt::Compound {
    use horizon_core::nbt::{Compound, Tag};
    let bees = ticks_in_hive
        .iter()
        .map(|&ticks| {
            let mut entity = Compound::new();
            entity.insert("id".into(), Tag::String("minecraft:bee".into()));
            let mut bee = Compound::new();
            bee.insert("entity_data".into(), Tag::Compound(entity));
            bee.insert("ticks_in_hive".into(), Tag::Int(ticks));
            bee.insert("min_ticks_in_hive".into(), Tag::Int(600));
            Tag::Compound(bee)
        })
        .collect();
    let mut nbt = Compound::new();
    nbt.insert("id".into(), Tag::String("minecraft:beehive".into()));
    nbt.insert("bees".into(), Tag::List(bees));
    nbt
}

#[cfg(test)]
mod tests {
    use super::super::Sets;
    use super::*;
    use crate::registry::BlockRegistry;
    use crate::worldgen::state_provider::StateProviderBindings;
    use crate::worldgen::{anchor::WorldGenerationContext, placement::PlacementQueryError};
    use horizon_core::random::LegacyRandom;
    use std::collections::{BTreeMap, HashMap};

    fn registry() -> &'static BlockRegistry {
        static REGISTRY: std::sync::OnceLock<BlockRegistry> = std::sync::OnceLock::new();
        REGISTRY.get_or_init(|| {
            let names = [
                "minecraft:air",
                "minecraft:grass_block",
                "minecraft:dirt",
                "minecraft:podzol",
                "minecraft:coarse_dirt",
                "minecraft:stone",
            ];
            let rows: Vec<_> = names
                .iter()
                .map(|name| {
                    serde_json::json!([name, name.ends_with("air"), false, "", {"id": name, "properties": {}}])
                })
                .collect();
            let defaults: BTreeMap<_, _> = names.iter().enumerate().map(|(i, n)| (*n, i)).collect();
            let facts = serde_json::json!({"states": rows, "defaults": defaults});
            BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap()
        })
    }
    fn state(name: &str) -> BlockStateId {
        registry().default_state(name).unwrap()
    }

    /// Grass at y 0 over dirt, air above: the ground a tree at y 1 stands on.
    struct Meadow(HashMap<[i32; 3], BlockStateId>);
    impl Meadow {
        fn new() -> Self {
            let mut blocks = HashMap::new();
            for x in -8..=8 {
                for z in -8..=8 {
                    blocks.insert([x, 0, z], state("minecraft:grass_block"));
                    for y in -3..0 {
                        blocks.insert([x, y, z], state("minecraft:dirt"));
                    }
                }
            }
            Self(blocks)
        }
        fn at(&self, pos: [i32; 3]) -> BlockStateId {
            self.0
                .get(&pos)
                .copied()
                .unwrap_or_else(|| state("minecraft:air"))
        }
    }
    impl FeatureWorld for Meadow {
        type Error = ();
        fn block(&mut self, pos: [i32; 3]) -> Result<BlockStateId, ()> {
            Ok(self.at(pos))
        }
        fn set_block(&mut self, pos: [i32; 3], state: BlockStateId, _: i32) -> Result<bool, ()> {
            self.0.insert(pos, state);
            Ok(true)
        }
    }
    impl PlacementContext for Meadow {
        fn block_state(&mut self, pos: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
            Ok(self.at(pos))
        }
        fn generation(&self) -> WorldGenerationContext {
            WorldGenerationContext::new(-64, 384, 63)
        }
        fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(-64)
        }
        fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
            Err(PlacementQueryError::Unavailable)
        }
    }

    fn bindings() -> StateProviderBindings<'static> {
        StateProviderBindings::new(registry(), Default::default(), &BTreeMap::new()).unwrap()
    }

    /// Mega spruce's kind of ground, with a draw per placed block: grass becomes podzol or
    /// coarse dirt.
    fn decorator_provider() -> BlockStateProvider {
        let json = serde_json::json!({
            "type": "minecraft:rule_based",
            "rules": [{
                "if_true": {"type": "minecraft:matching_blocks", "blocks": "minecraft:grass_block"},
                "then": {"type": "minecraft:weighted", "entries": [
                    {"data": {"id": "minecraft:podzol"}, "weight": 1},
                    {"data": {"id": "minecraft:coarse_dirt"}, "weight": 1},
                ]},
            }],
        });
        BlockStateProvider::from_json(&json.to_string(), &bindings()).unwrap()
    }

    /// The ground under a two-log trunk at x 0, z 0 altered from `seed`: the meadow after,
    /// and the random's next long.
    fn alter(seed: i64, replaced: Option<&dyn ModGroundProvider>) -> TreeResult<(Meadow, i64), ()> {
        let blocks = TreeBlocks::air_only(registry(), bindings().properties().unwrap());
        let decorator = decorator_provider();
        let mut world = Meadow::new();
        let mut random = LegacyRandom::new(seed);
        let mut run = Run {
            world: &mut world,
            random: &mut random,
            sets: Sets::default(),
            track: true,
        };
        let lists = Lists::new(vec![[0, 1, 0], [0, 2, 0]], vec![], vec![]);
        let ground = Ground {
            decorator: &decorator,
            replaced,
        };
        Decorate {
            run: &mut run,
            blocks: &blocks,
        }
        .alter_ground(&ground, &lists.lowest())?;
        Ok((world, random.next_long()))
    }

    fn changed(world: &Meadow) -> BTreeMap<[i32; 3], BlockStateId> {
        let before = Meadow::new();
        (world.0.iter())
            .filter(|(pos, state)| before.at(**pos) != **state)
            .map(|(pos, state)| (*pos, *state))
            .collect()
    }

    /// The grass the four fixed circles around a trunk at x 0, z 0 cover: each a 5x5 square
    /// without its corners (`AlterGroundDecorator.placeCircle`).
    fn fixed_circles() -> Vec<[i32; 3]> {
        let mut columns = Vec::new();
        for [cx, cz] in [[-1, -1], [2, -1], [-1, 2], [2, 2]] {
            for xx in -2i32..=2 {
                for zz in -2i32..=2 {
                    if xx.abs() != 2 || zz.abs() != 2 {
                        columns.push([cx + xx, 0, cz + zz]);
                    }
                }
            }
        }
        columns
    }

    /// Keeps the decorator's own provider, as a mod that wraps it and changes nothing.
    struct Kept;
    impl ModGroundProvider for Kept {
        fn state_at(&self, access: &mut dyn GroundAccess, pos: [i32; 3]) -> Option<BlockStateId> {
            access.decorator_state(pos)
        }
    }

    /// Turns grass to stone, drawing nothing.
    struct Stone;
    impl ModGroundProvider for Stone {
        fn state_at(&self, access: &mut dyn GroundAccess, pos: [i32; 3]) -> Option<BlockStateId> {
            let grass = access.block_state(pos) == state("minecraft:grass_block");
            grass.then(|| state("minecraft:stone"))
        }
    }

    #[test]
    fn the_decorators_own_provider_turns_the_circles_around_the_trunk() {
        let (world, _) = alter(7, None).unwrap();
        let changed = changed(&world);
        for column in fixed_circles() {
            assert!(changed.contains_key(&column), "{column:?} kept its grass");
        }
        let ground = [state("minecraft:podzol"), state("minecraft:coarse_dirt")];
        assert!(
            changed
                .iter()
                .all(|(pos, state)| pos[1] == 0 && ground.contains(state))
        );
        assert!(changed.values().any(|s| *s == ground[0]));
        assert!(changed.values().any(|s| *s == ground[1]));
    }

    #[test]
    fn a_provider_that_wraps_the_decorators_own_keeps_its_blocks_and_draws() {
        for seed in [7, -3, i64::MIN, i64::MAX] {
            let (own, own_next) = alter(seed, None).unwrap();
            let (kept, kept_next) = alter(seed, Some(&Kept)).unwrap();
            assert_eq!(changed(&own), changed(&kept), "seed {seed}");
            assert_eq!(own_next, kept_next, "seed {seed}");
        }
    }

    #[test]
    fn a_mods_provider_places_its_own_ground_and_draws_only_what_it_draws() {
        let (world, next) = alter(7, Some(&Stone)).unwrap();
        let changed = changed(&world);
        for column in fixed_circles() {
            assert_eq!(changed.get(&column), Some(&state("minecraft:stone")));
        }
        assert!(
            changed
                .iter()
                .all(|(pos, s)| pos[1] == 0 && *s == state("minecraft:stone"))
        );
        // The decorator itself draws five `nextInt(64)` for its one position; the provider
        // drew nothing.
        let mut expected = LegacyRandom::new(7);
        for _ in 0..5 {
            expected.next_int_bound(64).unwrap();
        }
        assert_eq!(next, expected.next_long());
    }

    #[test]
    fn a_state_the_registry_lacks_is_refused() {
        struct Unknown;
        impl ModGroundProvider for Unknown {
            fn state_at(&self, _: &mut dyn GroundAccess, _: [i32; 3]) -> Option<BlockStateId> {
                Some(BlockStateId(u32::MAX))
            }
        }
        assert!(matches!(
            alter(7, Some(&Unknown)),
            Err(FeatureExecutionError::InvalidState)
        ));
    }
}
