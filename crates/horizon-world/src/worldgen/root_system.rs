//! `minecraft:root_system`: a nested tree on a rooted-dirt column with
//! hanging roots below.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    block_predicate::BlockPredicate,
    feature::FeatureWorld,
    feature_graph::{FeatureExecutionError, PlacedFeatureDefinition},
    int_range,
    placement::{PlacementContext, PlacementQueryError},
    required,
    state_provider::BlockStateProvider,
    tree::{TreeDecoder, TreeResult, kind, read, sample_state, write},
    tree_blocks::{self as tb, TreeBlocks},
    vegetation::{PlacedDecoder, PlacedPlacer},
};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, random::RandomSource};
use std::{collections::BTreeSet, sync::Arc};

fn query<E>(error: PlacementQueryError) -> FeatureExecutionError<E> {
    FeatureExecutionError::Query(error)
}

#[derive(Clone, Debug)]
pub struct RootSystemFeature {
    tree: Box<PlacedFeatureDefinition>,
    required_vertical_space: i32,
    level_test_distance: i32,
    max_level_deviation: i32,
    root_radius: i32,
    root_replaceable: Arc<BTreeSet<BlockStateId>>,
    root_provider: BlockStateProvider,
    root_attempts: i32,
    root_column_max_height: i32,
    hanging_radius: i32,
    hanging_span: i32,
    hanging_provider: BlockStateProvider,
    hanging_attempts: i32,
    allowed_water: i32,
    allowed_tree_position: BlockPredicate,
    blocks: Arc<TreeBlocks>,
}
impl RootSystemFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
        nested: &PlacedDecoder<'_>,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:root_system" {
            return Err(DecodeError::Field("root system type"));
        }
        let blocks = bindings.tree_blocks()?;
        let tree = nested(required(&object, "feature")?.get(), depth + 1, remaining)?;
        let mut decoder = TreeDecoder {
            bindings,
            depth,
            remaining,
        };
        let int = |name: &'static str, low: i32, high: i32| {
            int_range(required(&object, name)?, name, low, high)
        };
        Ok(Self {
            tree: Box::new(tree),
            required_vertical_space: int("required_vertical_space_for_tree", 1, 64)?,
            level_test_distance: int("level_test_distance", 0, 16)?,
            max_level_deviation: int("max_level_deviation", 0, 64)?,
            root_radius: int("root_radius", 1, 64)?,
            root_replaceable: decoder.block_set(required(&object, "root_replaceable")?)?,
            root_provider: decoder.provider(required(&object, "root_state_provider")?)?,
            root_attempts: int("root_placement_attempts", 1, 256)?,
            root_column_max_height: int("root_column_max_height", 1, 4096)?,
            hanging_radius: int("hanging_root_radius", 1, 64)?,
            hanging_span: int("hanging_roots_vertical_span", 1, 16)?,
            hanging_provider: decoder
                .provider(required(&object, "hanging_root_state_provider")?)?,
            hanging_attempts: int("hanging_root_placement_attempts", 1, 256)?,
            allowed_water: int("allowed_vertical_water_for_tree", 1, 64)?,
            allowed_tree_position: BlockPredicate::decode_with_blocks(
                required(&object, "allowed_tree_position")?.get(),
                depth + 1,
                decoder.remaining,
                Some(bindings),
            )?,
            blocks,
        })
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        nested: &mut PlacedPlacer<'_, W, R>,
    ) -> TreeResult<bool, W::Error> {
        if !self.blocks.is_air(read(world, origin)?) {
            return Ok(false);
        }
        if self.dirt_and_tree(world, random, origin, nested)? {
            self.hanging_roots(world, random, origin)?;
        }
        Ok(true)
    }
    fn space_for_tree<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        pos: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let mut column = pos;
        for i in 1..=self.required_vertical_space {
            column = tb::offset(column, 0, 1, 0);
            let state = read(world, column)?;
            let allowed = self.blocks.is_air(state)
                || (i < self.allowed_water && self.blocks.has(state, tb::WATER_FLUID));
            if !allowed {
                return Ok(false);
            }
        }
        if self.level_test_distance > 0 {
            // Direction.from2DDataValue: SOUTH, WEST, NORTH, EAST.
            for direction in [
                Direction::South,
                Direction::West,
                Direction::North,
                Direction::East,
            ] {
                let corner = tb::relative(pos, direction, self.level_test_distance);
                let below = read(world, tb::offset(corner, 0, -self.max_level_deviation, 0))?;
                let above = read(world, tb::offset(corner, 0, self.max_level_deviation, 0))?;
                if self.blocks.is_air(below) || !self.blocks.is_air(above) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    fn dirt_and_tree<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        nested: &mut PlacedPlacer<'_, W, R>,
    ) -> TreeResult<bool, W::Error> {
        let mut working = origin;
        for y in 0..self.root_column_max_height {
            working = tb::offset(working, 0, 1, 0);
            let surface = world
                .height(
                    crate::heightmap::HeightmapType::WorldSurface,
                    working[0],
                    working[2],
                )
                .map_err(query)?;
            if surface < working[1] {
                return Ok(false);
            }
            if self
                .allowed_tree_position
                .test(world, working)
                .map_err(query)?
                && self.space_for_tree(world, working)?
            {
                let below_state = read(world, tb::offset(working, 0, -1, 0))?;
                if self.blocks.has(below_state, tb::LAVA_FLUID)
                    || !self.blocks.has(below_state, tb::SOLID)
                {
                    return Ok(false);
                }
                if nested(&self.tree, world, random, working)? {
                    let target = origin[1].wrapping_add(y);
                    let mut level = origin[1];
                    while level < target {
                        self.rooted_dirt(world, random, [origin[0], level, origin[2]])?;
                        level += 1;
                    }
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn rooted_dirt<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        column: [i32; 3],
    ) -> TreeResult<(), W::Error> {
        let r = self.root_radius;
        for _ in 0..self.root_attempts {
            let dx = random.next_int_bound(r)? - random.next_int_bound(r)?;
            let dz = random.next_int_bound(r)? - random.next_int_bound(r)?;
            let p = tb::offset(column, dx, 0, dz);
            if self.root_replaceable.contains(&read(world, p)?) {
                let state = sample_state(&self.root_provider, world, random, p)?;
                write(world, p, state, 2)?;
            }
        }
        Ok(())
    }
    fn hanging_roots<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<(), W::Error> {
        let (r, span) = (self.hanging_radius, self.hanging_span);
        for _ in 0..self.hanging_attempts {
            let dx = random.next_int_bound(r)? - random.next_int_bound(r)?;
            let dy = random.next_int_bound(span)? - random.next_int_bound(span)?;
            let dz = random.next_int_bound(r)? - random.next_int_bound(r)?;
            let p = tb::offset(origin, dx, dy, dz);
            if !self.blocks.is_air(read(world, p)?) {
                continue;
            }
            let state = sample_state(&self.hanging_provider, world, random, p)?;
            if world.block_survives(state, p).map_err(query)? {
                let above = read(world, tb::offset(p, 0, 1, 0))?;
                if world
                    .state_face_sturdy(above, p, Direction::Down)
                    .map_err(query)?
                {
                    write(world, p, state, 2)?;
                }
            }
        }
        Ok(())
    }
}
