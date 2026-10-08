//! `minecraft:sculk_patch`: the world-generation SculkSpreader with sculk,
//! sculk vein and default charge-cursor behaviours.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    feature_graph::FeatureExecutionError,
    int_range,
    placement::{PlacementContext, PlacementQueryError},
    required,
    tree::{TreeResult, kind, read, write},
    tree_blocks::{self as tb, TreeBlocks},
};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, random::RandomSource};
use std::{collections::BTreeSet, sync::Arc};

fn query<E>(error: PlacementQueryError) -> FeatureExecutionError<E> {
    FeatureExecutionError::Query(error)
}

// Worldgen spreader constants (SculkSpreader.createWorldGenSpreader).
const GROWTH_SPAWN_COST: i32 = 50;
const NO_GROWTH_RADIUS: i32 = 1;
const CHARGE_DECAY_RATE: i32 = 5;
const ADDITIONAL_DECAY_RATE: i32 = 10;
const MAX_CURSORS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    Default,
    Sculk,
    Vein,
}
#[derive(Clone, Copy)]
enum Spread {
    SamePosition,
    SamePlane,
    WrapAround,
}
const ALL_SPREADS: [Spread; 3] = [Spread::SamePosition, Spread::SamePlane, Spread::WrapAround];

struct Cursor {
    pos: [i32; 3],
    charge: i32,
    update_delay: i32,
    decay_delay: i32,
    facings: Option<Vec<Direction>>,
}

#[derive(Clone, Debug)]
pub struct SculkPatchFeature {
    charge_count: i32,
    amount_per_charge: i32,
    spread_attempts: i32,
    growth_rounds: i32,
    spread_rounds: i32,
    sculk: BlockStateId,
    vein: BlockStateId,
    shrieker: BlockStateId,
    sensor: BlockStateId,
    water: BlockStateId,
    air: BlockStateId,
    replaceable_world_gen: Arc<BTreeSet<BlockStateId>>,
    replaceable: Arc<BTreeSet<BlockStateId>>,
    inhibitors: Arc<BTreeSet<BlockStateId>>,
    blocks: Arc<TreeBlocks>,
}
impl SculkPatchFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:sculk_patch" {
            return Err(DecodeError::Field("sculk patch type"));
        }
        let int = |name: &'static str, low: i32, high: i32| {
            int_range(required(&object, name)?, name, low, high)
        };
        let registry = bindings.registry();
        let default = |name: &str| {
            registry
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("missing sculk block {name}")))
        };
        let tag = |name: &str| -> Arc<BTreeSet<BlockStateId>> {
            bindings.tags.get(name).cloned().unwrap_or_default()
        };
        let blocks = bindings.tree_blocks()?;
        let shrieker = blocks
            .with(default("minecraft:sculk_shrieker")?, "can_summon", "true")
            .ok_or(DecodeError::Field("sculk shrieker summon state"))?;
        Ok(Self {
            charge_count: int("charge_count", 1, 32)?,
            amount_per_charge: int("amount_per_charge", 1, 500)?,
            spread_attempts: int("spread_attempts", 1, 64)?,
            growth_rounds: int("growth_rounds", 0, 8)?,
            spread_rounds: int("spread_rounds", 0, 8)?,
            sculk: default("minecraft:sculk")?,
            vein: default("minecraft:sculk_vein")?,
            shrieker,
            sensor: default("minecraft:sculk_sensor")?,
            water: default("minecraft:water")?,
            air: default("minecraft:air")?,
            replaceable_world_gen: tag("minecraft:sculk_replaceable_world_gen"),
            replaceable: tag("minecraft:sculk_replaceable"),
            inhibitors: tag("minecraft:sculk_growth_inhibitors"),
            blocks,
        })
    }
    fn behaviour(&self, state: BlockStateId) -> Behaviour {
        if self.blocks.same_block(state, self.sculk) {
            Behaviour::Sculk
        } else if self.blocks.same_block(state, self.vein) {
            Behaviour::Vein
        } else {
            Behaviour::Default
        }
    }
    fn has_face(&self, state: BlockStateId, face: Direction) -> bool {
        self.blocks
            .properties
            .value(state, tb::direction_name(face))
            == Some("true")
    }
    fn is_vein(&self, state: BlockStateId) -> bool {
        self.blocks.same_block(state, self.vein)
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        if !self.can_spread_from(world, origin)? {
            return Ok(false);
        }
        let mut cursors: Vec<Cursor> = Vec::new();
        for round in 0..self.spread_rounds + self.growth_rounds {
            for _ in 0..self.charge_count {
                let mut charge = self.amount_per_charge;
                while charge > 0 {
                    let current = charge.min(1000);
                    if cursors.len() < MAX_CURSORS {
                        cursors.push(Cursor {
                            pos: origin,
                            charge: current,
                            update_delay: 0,
                            decay_delay: 1,
                            facings: None,
                        });
                    }
                    charge -= current;
                }
            }
            let spread_veins = round < self.spread_rounds;
            for _ in 0..self.spread_attempts {
                cursors = self.update_cursors(world, random, origin, cursors, spread_veins)?;
            }
            cursors.clear();
        }
        Ok(true)
    }
    fn can_spread_from<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let start = read(world, origin)?;
        if self.behaviour(start) != Behaviour::Default {
            return Ok(true);
        }
        let water_source =
            self.blocks.has(start, tb::WATER_BLOCK) && self.blocks.has(start, tb::FLUID_SOURCE);
        if !self.blocks.is_air(start) && !water_source {
            return Ok(false);
        }
        for direction in Direction::ALL {
            let p = tb::relative(origin, direction, 1);
            let state = read(world, p)?;
            if world.collision_full_block(state, p).map_err(query)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn update_cursors<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        cursors: Vec<Cursor>,
        spread_veins: bool,
    ) -> TreeResult<Vec<Cursor>, W::Error> {
        let mut processed: Vec<Cursor> = Vec::new();
        // Mergeable cursor charge per position (index into `processed`).
        let mut mergeable: Vec<([i32; 3], usize, i32)> = Vec::new();
        for mut cursor in cursors {
            let distance = (0..3)
                .map(|i| cursor.pos[i].wrapping_sub(origin[i]).wrapping_abs())
                .max()
                .unwrap_or(0);
            if distance > 1024 {
                continue;
            }
            self.update(world, random, origin, &mut cursor, spread_veins)?;
            if cursor.charge <= 0 {
                continue;
            }
            match mergeable.iter_mut().find(|(p, _, _)| *p == cursor.pos) {
                None => {
                    mergeable.push((cursor.pos, processed.len(), cursor.charge));
                    processed.push(cursor);
                }
                Some(entry) => {
                    // World generation never merges cursors.
                    if cursor.charge < entry.2 {
                        *entry = (cursor.pos, processed.len(), cursor.charge);
                    }
                    processed.push(cursor);
                }
            }
        }
        Ok(processed)
    }
    fn update<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        cursor: &mut Cursor,
        spread_veins: bool,
    ) -> TreeResult<(), W::Error> {
        if cursor.charge <= 0 {
            return Ok(());
        }
        if cursor.update_delay > 0 {
            cursor.update_delay -= 1;
            return Ok(());
        }
        let mut state = read(world, cursor.pos)?;
        let mut behaviour = self.behaviour(state);
        if spread_veins
            && self.attempt_spread_vein(
                world,
                behaviour,
                cursor.pos,
                state,
                cursor.facings.as_deref(),
            )?
            && behaviour != Behaviour::Sculk
        {
            state = read(world, cursor.pos)?;
            behaviour = self.behaviour(state);
        }
        cursor.charge =
            self.attempt_use_charge(world, random, origin, behaviour, cursor, spread_veins)?;
        if cursor.charge <= 0 {
            self.on_discharged(world, behaviour, state, cursor.pos)?;
            return Ok(());
        }
        match self.valid_movement(world, random, cursor.pos, origin)? {
            Some(target) => {
                self.on_discharged(world, behaviour, state, cursor.pos)?;
                cursor.pos = target;
                state = read(world, target)?;
            }
            None => {
                self.on_discharged(world, behaviour, state, cursor.pos)?;
                cursor.charge = 0;
                return Ok(());
            }
        }
        if self.behaviour(state) != Behaviour::Default {
            cursor.facings = Some(
                Direction::ALL
                    .into_iter()
                    .filter(|d| self.has_face(state, *d))
                    .collect(),
            );
        }
        cursor.decay_delay = match behaviour {
            Behaviour::Default => (cursor.decay_delay - 1).max(0),
            _ => 1,
        };
        cursor.update_delay = 1;
        Ok(())
    }
    fn attempt_spread_vein<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        behaviour: Behaviour,
        pos: [i32; 3],
        state: BlockStateId,
        facings: Option<&[Direction]>,
    ) -> TreeResult<bool, W::Error> {
        if behaviour == Behaviour::Default {
            match facings {
                None => {
                    let current = read(world, pos)?;
                    return Ok(self.spread_all(world, current, pos, &[Spread::SamePosition])? > 0);
                }
                Some(faces) if !faces.is_empty() => {
                    let water_source = self.blocks.has(state, tb::WATER_SOURCE);
                    if !self.blocks.is_air(state) && !water_source {
                        return Ok(false);
                    }
                    return self.regrow(world, pos, state, faces);
                }
                Some(_) => {}
            }
        }
        Ok(self.spread_all(world, state, pos, &ALL_SPREADS)? > 0)
    }
    fn regrow<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        pos: [i32; 3],
        existing: BlockStateId,
        faces: &[Direction],
    ) -> TreeResult<bool, W::Error> {
        let mut state = self.vein;
        let mut any = false;
        for &face in faces {
            if world.attachment_at(pos, face).map_err(query)? {
                state = self.with_face(state, face, true)?;
                any = true;
            }
        }
        if !any {
            return Ok(false);
        }
        if self.blocks.has(existing, tb::ANY_FLUID) {
            state = self.blocks.waterlogged(state, true);
        }
        write(world, pos, state, 3)?;
        Ok(true)
    }
    fn with_face<E>(
        &self,
        state: BlockStateId,
        face: Direction,
        value: bool,
    ) -> TreeResult<BlockStateId, E> {
        self.blocks
            .with(
                state,
                tb::direction_name(face),
                if value { "true" } else { "false" },
            )
            .ok_or(FeatureExecutionError::InvalidState)
    }
    /// MultifaceSpreader.spreadAll with the sculk vein spreader configuration
    /// (post-processing enabled for world generation).
    fn spread_all<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        state: BlockStateId,
        pos: [i32; 3],
        types: &[Spread],
    ) -> TreeResult<i64, W::Error> {
        let other_source = !self.is_vein(state);
        let mut count = 0;
        for face in Direction::ALL {
            if !(other_source || self.has_face(state, face)) {
                continue;
            }
            for direction in Direction::ALL {
                if tb::Axis::of(direction) == tb::Axis::of(face) {
                    continue;
                }
                if !(other_source
                    || (self.has_face(state, face) && !self.has_face(state, direction)))
                {
                    continue;
                }
                let mut target = None;
                for kind in types {
                    let candidate = match kind {
                        Spread::SamePosition => (pos, direction),
                        Spread::SamePlane => (tb::relative(pos, direction, 1), face),
                        Spread::WrapAround => (
                            tb::relative(tb::relative(pos, direction, 1), face, 1),
                            direction.opposite(),
                        ),
                    };
                    if self.can_spread_into(world, pos, candidate.0, candidate.1)? {
                        target = Some(candidate);
                        break;
                    }
                }
                let Some((target, face_to)) = target else {
                    continue;
                };
                let old = read(world, target)?;
                let Some(new) = self.state_for_placement(world, old, target, face_to)? else {
                    continue;
                };
                world.mark_postprocessing(target).map_err(query)?;
                if write(world, target, new, 2)? {
                    count += 1;
                }
            }
        }
        Ok(count)
    }
    fn valid_for_placement<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        old: BlockStateId,
        pos: [i32; 3],
        face: Direction,
    ) -> TreeResult<bool, W::Error> {
        if self.is_vein(old) && self.has_face(old, face) {
            return Ok(false);
        }
        world.attachment_at(pos, face).map_err(query)
    }
    fn state_for_placement<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        old: BlockStateId,
        pos: [i32; 3],
        face: Direction,
    ) -> TreeResult<Option<BlockStateId>, W::Error> {
        if !self.valid_for_placement(world, old, pos, face)? {
            return Ok(None);
        }
        let base = if self.is_vein(old) {
            old
        } else if self.blocks.has(old, tb::WATER_SOURCE) {
            self.blocks.waterlogged(self.vein, true)
        } else {
            self.vein
        };
        self.with_face(base, face, true).map(Some)
    }
    fn can_spread_into<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        source: [i32; 3],
        target: [i32; 3],
        face: Direction,
    ) -> TreeResult<bool, W::Error> {
        let existing = read(world, target)?;
        let against = read(world, tb::relative(target, face, 1))?;
        if self.blocks.has(against, tb::SCULK_BLOCKER) {
            return Ok(false);
        }
        if tb::dist_manhattan(source, target) == 2 {
            let neighbor = tb::relative(source, face.opposite(), 1);
            let state = read(world, neighbor)?;
            if world
                .state_face_sturdy(state, neighbor, face)
                .map_err(query)?
            {
                return Ok(false);
            }
        }
        if self.blocks.has(existing, tb::ANY_FLUID) && !self.blocks.has(existing, tb::WATER_SOURCE)
        {
            return Ok(false);
        }
        if self.blocks.has(existing, tb::FIRE) {
            return Ok(false);
        }
        let replaceable = self.blocks.has(existing, tb::REPLACEABLE)
            || self.blocks.is_air(existing)
            || self.is_vein(existing)
            || (self.blocks.has(existing, tb::WATER_BLOCK)
                && self.blocks.has(existing, tb::WATER_SOURCE));
        Ok(replaceable && self.valid_for_placement(world, existing, target, face)?)
    }
    fn attempt_use_charge<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        behaviour: Behaviour,
        cursor: &Cursor,
        spread_veins: bool,
    ) -> TreeResult<i32, W::Error> {
        let charge = cursor.charge;
        match behaviour {
            Behaviour::Default => Ok(if cursor.decay_delay > 0 { charge } else { 0 }),
            Behaviour::Vein => {
                if spread_veins && self.place_sculk(world, random, cursor.pos)? {
                    Ok(charge - 1)
                } else if random.next_int_bound(CHARGE_DECAY_RATE)? == 0 {
                    Ok((charge as f32 * 0.5f32).floor() as i32)
                } else {
                    Ok(charge)
                }
            }
            Behaviour::Sculk => {
                if charge == 0 || random.next_int_bound(CHARGE_DECAY_RATE)? != 0 {
                    return Ok(charge);
                }
                let pos = cursor.pos;
                let dist_sqr: f64 = (0..3)
                    .map(|i| {
                        let d = f64::from(pos[i]) - f64::from(origin[i]);
                        d * d
                    })
                    .sum();
                let close = dist_sqr < f64::from(NO_GROWTH_RADIUS * NO_GROWTH_RADIUS);
                if !close && self.can_place_growth(world, pos)? {
                    if random.next_int_bound(GROWTH_SPAWN_COST)? < charge {
                        let above = tb::offset(pos, 0, 1, 0);
                        let mut growth = if random.next_int_bound(11)? == 0 {
                            self.shrieker
                        } else {
                            self.sensor
                        };
                        if self.blocks.has(growth, tb::WATERLOGGABLE)
                            && self.blocks.has(read(world, above)?, tb::ANY_FLUID)
                        {
                            growth = self.blocks.waterlogged(growth, true);
                        }
                        write(world, above, growth, 3)?;
                    }
                    return Ok(0.max(charge - GROWTH_SPAWN_COST));
                }
                if random.next_int_bound(ADDITIONAL_DECAY_RATE)? != 0 {
                    return Ok(charge);
                }
                let penalty = if close {
                    1
                } else {
                    let outer = (dist_sqr.sqrt() as f32 - NO_GROWTH_RADIUS as f32).powi(2);
                    let reach = (24 - NO_GROWTH_RADIUS) * (24 - NO_GROWTH_RADIUS);
                    let factor = 1.0f32.min(outer / reach as f32);
                    1.max((charge as f32 * factor * 0.5f32) as i32)
                };
                Ok(charge - penalty)
            }
        }
    }
    fn can_place_growth<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        pos: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let above = read(world, tb::offset(pos, 0, 1, 0))?;
        let open = self.blocks.is_air(above)
            || (self.blocks.has(above, tb::WATER_BLOCK)
                && self.blocks.has(above, tb::WATER_SOURCE));
        if !open {
            return Ok(false);
        }
        let mut matched = 0;
        // BoxBlockMatcher iterates BlockPos.betweenClosed (x, then y, then z).
        for dz in -4..=4 {
            for dy in 0..=2 {
                for dx in -4..=4 {
                    if self
                        .inhibitors
                        .contains(&read(world, tb::offset(pos, dx, dy, dz))?)
                    {
                        matched += 1;
                        if matched >= 3 {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        Ok(true)
    }
    fn place_sculk<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        pos: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let state = read(world, pos)?;
        let mut directions = Direction::ALL;
        tb::shuffle(&mut directions, random)?;
        for support in directions {
            if !self.has_face(state, support) {
                continue;
            }
            let support_pos = tb::relative(pos, support, 1);
            let support_state = read(world, support_pos)?;
            if !self.replaceable_world_gen.contains(&support_state) {
                continue;
            }
            write(world, support_pos, self.sculk, 3)?;
            self.spread_all(world, self.sculk, support_pos, &ALL_SPREADS)?;
            let skip = support.opposite();
            for direction in Direction::ALL {
                if direction == skip {
                    continue;
                }
                let vein_pos = tb::relative(support_pos, direction, 1);
                let possible = read(world, vein_pos)?;
                if self.is_vein(possible) {
                    self.on_discharged(world, Behaviour::Vein, possible, vein_pos)?;
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
    fn on_discharged<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        behaviour: Behaviour,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> TreeResult<(), W::Error> {
        if behaviour != Behaviour::Vein || !self.is_vein(state) {
            return Ok(());
        }
        let mut state = state;
        for direction in Direction::ALL {
            if self.has_face(state, direction)
                && self
                    .blocks
                    .same_block(read(world, tb::relative(pos, direction, 1))?, self.sculk)
            {
                state = self.with_face(state, direction, false)?;
            }
        }
        if !Direction::ALL.into_iter().any(|d| self.has_face(state, d)) {
            let here = read(world, pos)?;
            state = if self.blocks.has(here, tb::ANY_FLUID) {
                self.water
            } else {
                self.air
            };
        }
        write(world, pos, state, 3)?;
        Ok(())
    }
    fn valid_movement<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        pos: [i32; 3],
        origin: [i32; 3],
    ) -> TreeResult<Option<[i32; 3]>, W::Error> {
        // NON_CORNER_NEIGHBOURS in betweenClosed order (x, then y, then z).
        let mut offsets = Vec::with_capacity(18);
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    if (x == 0 || y == 0 || z == 0) && (x, y, z) != (0, 0, 0) {
                        offsets.push([x, y, z]);
                    }
                }
            }
        }
        tb::shuffle(&mut offsets, random)?;
        let mut best = pos;
        for o in offsets {
            let neighbour = tb::offset(pos, o[0], o[1], o[2]);
            let dx = origin[0].wrapping_sub(neighbour[0]);
            let dz = origin[2].wrapping_sub(neighbour[2]);
            if dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) > 144 {
                continue;
            }
            let transferee = read(world, neighbour)?;
            if self.behaviour(transferee) == Behaviour::Default
                || !self.unobstructed_move(world, pos, neighbour)?
            {
                continue;
            }
            best = neighbour;
            if self.is_vein(transferee) {
                let mut substrate = false;
                for direction in Direction::ALL {
                    if self.has_face(transferee, direction)
                        && self
                            .replaceable
                            .contains(&read(world, tb::relative(neighbour, direction, 1))?)
                    {
                        substrate = true;
                        break;
                    }
                }
                if substrate {
                    break;
                }
            }
        }
        Ok((best != pos).then_some(best))
    }
    fn unobstructed_move<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        from: [i32; 3],
        to: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        if tb::dist_manhattan(from, to) == 1 {
            return Ok(true);
        }
        let delta: [i32; 3] = std::array::from_fn(|i| to[i].wrapping_sub(from[i]));
        let pick = |v: i32, negative: Direction, positive: Direction| {
            if v < 0 { negative } else { positive }
        };
        let dir_x = pick(delta[0], Direction::West, Direction::East);
        let dir_y = pick(delta[1], Direction::Down, Direction::Up);
        let dir_z = pick(delta[2], Direction::North, Direction::South);
        let (a, b) = if delta[0] == 0 {
            (dir_y, dir_z)
        } else if delta[1] == 0 {
            (dir_x, dir_z)
        } else {
            (dir_x, dir_y)
        };
        Ok(self.unobstructed(world, from, a)? || self.unobstructed(world, from, b)?)
    }
    fn unobstructed<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        from: [i32; 3],
        direction: Direction,
    ) -> TreeResult<bool, W::Error> {
        let test = tb::relative(from, direction, 1);
        let state = read(world, test)?;
        Ok(!world
            .state_face_sturdy(state, test, direction.opposite())
            .map_err(query)?)
    }
}
