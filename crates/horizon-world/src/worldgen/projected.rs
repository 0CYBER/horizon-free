//! 26.3 projection-style features: `minecraft:projected_random_patchy_square`
//! and `minecraft:stepped_column_cluster` (basalt columns).
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    block_predicate::BlockPredicate,
    feature::FeatureWorld,
    feature_graph::FeatureExecutionError,
    int_range,
    placement::{PlacementContext, PlacementQueryError},
    required,
    state_provider::BlockStateProvider,
    tree::{TreeDecoder, TreeResult, kind, read, sample_optional, sample_state, write},
    tree_blocks::{self as tb, TreeBlocks},
    value_provider::IntProvider,
};
use crate::registry::BlockStateId;
use horizon_core::random::RandomSource;
use std::{collections::BTreeSet, sync::Arc};

fn query<E>(error: PlacementQueryError) -> FeatureExecutionError<E> {
    FeatureExecutionError::Query(error)
}
fn predicate(
    object: &Object,
    name: &'static str,
    bindings: &FeatureBlockBindings<'_>,
    depth: usize,
    remaining: &mut usize,
) -> Result<BlockPredicate, DecodeError> {
    BlockPredicate::decode_with_blocks(
        required(object, name)?.get(),
        depth + 1,
        remaining,
        Some(bindings),
    )
}

#[derive(Clone, Debug)]
pub struct ProjectedPatchySquareFeature {
    block: BlockStateProvider,
    project_through: BlockPredicate,
    size: IntProvider,
    max_projection_height: i32,
}
impl ProjectedPatchySquareFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:projected_random_patchy_square" {
            return Err(DecodeError::Field("projected patchy square type"));
        }
        let mut decoder = TreeDecoder {
            bindings,
            depth,
            remaining,
        };
        let block = decoder.provider(required(&object, "block")?)?;
        let size = decoder.int_provider(required(&object, "size")?, 1, 16)?;
        Ok(Self {
            block,
            size,
            project_through: predicate(&object, "project_through", bindings, depth, remaining)?,
            max_projection_height: int_range(
                required(&object, "max_projection_height")?,
                "max_projection_height",
                0,
                i32::MAX,
            )?,
        })
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let size = self.size.sample(random)?;
        let bound = size.wrapping_mul(size).wrapping_add(1);
        for dx in -size..=size {
            for dz in -size..=size {
                let probability = dx.abs().wrapping_mul(dz.abs());
                if random.next_int_bound(bound)? < bound.wrapping_sub(probability) {
                    let mut base = tb::offset(origin, dx, 0, dz);
                    let mut drop = self.max_projection_height;
                    while self
                        .project_through
                        .test(world, tb::offset(base, 0, -1, 0))
                        .map_err(query)?
                    {
                        base = tb::offset(base, 0, -1, 0);
                        drop -= 1;
                        if drop <= 0 {
                            break;
                        }
                    }
                    if let Some(state) = sample_optional(&self.block, world, random, base)? {
                        write(world, base, state, 2)?;
                    }
                }
            }
        }
        Ok(true)
    }
}

#[derive(Clone, Debug)]
pub struct SteppedColumnClusterFeature {
    block: BlockStateProvider,
    continue_through: BlockPredicate,
    can_replace: BlockPredicate,
    cannot_place_on: Arc<BTreeSet<BlockStateId>>,
    cluster_reach: IntProvider,
    column_count: IntProvider,
    column_reach: IntProvider,
    height: IntProvider,
    blocks: Arc<TreeBlocks>,
}
impl SteppedColumnClusterFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:stepped_column_cluster" {
            return Err(DecodeError::Field("stepped column cluster type"));
        }
        let blocks = bindings.tree_blocks()?;
        let mut decoder = TreeDecoder {
            bindings,
            depth,
            remaining,
        };
        let block = decoder.provider(required(&object, "block")?)?;
        let cannot_place_on = decoder.block_set(required(&object, "cannot_place_on")?)?;
        let cluster_reach = decoder.int_provider(required(&object, "cluster_reach")?, 0, 13)?;
        let column_count = decoder.int_provider(required(&object, "column_count")?, 1, 150)?;
        let column_reach = decoder.int_provider(required(&object, "column_reach")?, 0, 3)?;
        let height = decoder.int_provider(required(&object, "height")?, 1, 10)?;
        Ok(Self {
            block,
            continue_through: predicate(&object, "continue_through", bindings, depth, remaining)?,
            can_replace: predicate(&object, "can_replace", bindings, depth, remaining)?,
            cannot_place_on,
            cluster_reach,
            column_count,
            column_reach,
            height,
            blocks,
        })
    }
    fn can_place_at<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        if !self.can_replace.test(world, p).map_err(query)? {
            return Ok(false);
        }
        let below = read(world, tb::offset(p, 0, -1, 0))?;
        Ok(!self.blocks.is_air(below) && !self.cannot_place_on.contains(&below))
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        if !self.can_place_at(world, origin)? {
            return Ok(false);
        }
        let column_height = self.height.sample(random)?;
        let reach = column_height.min(self.cluster_reach.sample(random)?);
        let count = self.column_count.sample(random)?;
        let width = reach.wrapping_mul(2).wrapping_add(1);
        let mut placed = false;
        for _ in 0..count {
            let x = origin[0]
                .wrapping_sub(reach)
                .wrapping_add(random.next_int_bound(width)?);
            let y = origin[1].wrapping_add(random.next_int_bound(1)?);
            let z = origin[2]
                .wrapping_sub(reach)
                .wrapping_add(random.next_int_bound(width)?);
            let pos = [x, y, z];
            let blocks_y = column_height - tb::dist_manhattan(pos, origin);
            if blocks_y >= 0 {
                let column_reach = self.column_reach.sample(random)?;
                placed |= self.place_column(world, random, pos, blocks_y, column_reach)?;
            }
        }
        Ok(placed)
    }
    fn place_column<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        column_height: i32,
        reach: i32,
    ) -> TreeResult<bool, W::Error> {
        let mut placed = false;
        // BlockPos.betweenClosed over a single layer: x fastest, then z.
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let pos = tb::offset(origin, dx, 0, dz);
                let step_limit = tb::dist_manhattan(pos, origin);
                let column = if self.can_replace.test(world, pos).map_err(query)? {
                    self.find_surface(world, pos, step_limit)?
                } else {
                    self.find_air(world, pos, step_limit)?
                };
                let Some(mut cursor) = column else {
                    continue;
                };
                let mut blocks_y = column_height - step_limit / 2;
                while blocks_y >= 0 {
                    if self.can_replace.test(world, cursor).map_err(query)? {
                        let state = sample_state(&self.block, world, random, cursor)?;
                        write(world, cursor, state, 3)?;
                        cursor = tb::offset(cursor, 0, 1, 0);
                        placed = true;
                    } else {
                        if !self.continue_through.test(world, cursor).map_err(query)? {
                            break;
                        }
                        cursor = tb::offset(cursor, 0, 1, 0);
                    }
                    blocks_y -= 1;
                }
            }
        }
        Ok(placed)
    }
    fn find_surface<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        mut cursor: [i32; 3],
        mut limit: i32,
    ) -> TreeResult<Option<[i32; 3]>, W::Error> {
        let min_y = world.min_y().map_err(query)?;
        while cursor[1] > min_y.wrapping_add(1) && limit > 0 {
            limit -= 1;
            if self.can_place_at(world, cursor)? {
                return Ok(Some(cursor));
            }
            cursor = tb::offset(cursor, 0, -1, 0);
        }
        Ok(None)
    }
    fn find_air<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        mut cursor: [i32; 3],
        mut limit: i32,
    ) -> TreeResult<Option<[i32; 3]>, W::Error> {
        let max_y = world.max_y().map_err(query)?;
        while cursor[1] <= max_y && limit > 0 {
            limit -= 1;
            let state = read(world, cursor)?;
            if self.cannot_place_on.contains(&state) {
                return Ok(None);
            }
            if self.blocks.is_air(state) {
                return Ok(Some(cursor));
            }
            cursor = tb::offset(cursor, 0, 1, 0);
        }
        Ok(None)
    }
}
