//! Randomized two-layer piles, with native support checks and write flags.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    block_predicate::PredicateBlockResolver,
    feature::FeatureWorld,
    placement::{PlacementContext, PlacementQueryError},
    required,
    state_provider::{BlockStateProvider, StateProviderError},
};
use crate::registry::BlockStateId;
use horizon_core::{
    geometry::Direction,
    random::{InvalidBound, RandomSource},
};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Debug)]
pub struct BlockPileFeature {
    provider: BlockStateProvider,
    air: [BlockStateId; 3],
    paths: Arc<BTreeSet<BlockStateId>>,
}
#[derive(Debug)]
pub enum BlockPileError<E> {
    Random(InvalidBound),
    Provider(StateProviderError<E>),
    Query(PlacementQueryError),
    World(E),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for BlockPileError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Random(e) => e.fmt(f),
            Self::Provider(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("block pile work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BlockPileError<E> {}
impl BlockPileFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        Self::decode(json, bindings, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        if depth > 128 || *remaining == 0 {
            return Err(DecodeError::Field("block pile depth or work"));
        }
        *remaining -= 1;
        let object: Object = serde_json::from_str(json)?;
        let (providers, catalog, _) = bindings
            .simple_blocks
            .as_ref()
            .ok_or(DecodeError::Field("block pile provider bindings"))?;
        Ok(Self {
            provider: catalog.provider_with_budget(
                required(&object, "state_provider")?.get(),
                providers,
                depth + 1,
                remaining,
            )?,
            air: bindings.air,
            paths: bindings.predicate_block_set(r#""minecraft:dirt_path""#)?,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, BlockPileError<W::Error>> {
        if origin[1]
            < world
                .min_y()
                .map_err(BlockPileError::Query)?
                .wrapping_add(5)
        {
            return Ok(false);
        }
        let rx = 2 + random.next_int_bound(2).map_err(BlockPileError::Random)?;
        let rz = 2 + random.next_int_bound(2).map_err(BlockPileError::Random)?;
        let a = [
            origin[0].wrapping_sub(rx),
            origin[1],
            origin[2].wrapping_sub(rz),
        ];
        let b = [
            origin[0].wrapping_add(rx),
            origin[1].wrapping_add(1),
            origin[2].wrapping_add(rz),
        ];
        let min: [i32; 3] = std::array::from_fn(|i| a[i].min(b[i]));
        let sizes: [i32; 3] =
            std::array::from_fn(|i| a[i].max(b[i]).wrapping_sub(min[i]).wrapping_add(1));
        let count = sizes[0].wrapping_mul(sizes[1]).wrapping_mul(sizes[2]);
        let mut index: i32 = 0;
        // Native betweenClosed stops on equality, including overflowed counts.
        // Ordinary piles visit at most 98 cells; pathological boxes are bounded.
        while index != count {
            if index == 1_048_576 {
                return Err(BlockPileError::WorkLimit);
            }
            let quotient = index / sizes[0];
            let p = [
                min[0].wrapping_add(index % sizes[0]),
                min[1].wrapping_add(quotient % sizes[1]),
                min[2].wrapping_add(quotient / sizes[1]),
            ];
            index += 1;
            let dx = origin[0].wrapping_sub(p[0]);
            let dz = origin[2].wrapping_sub(p[2]);
            let distance = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) as f32;
            let threshold = random.next_float() * 10.0 - random.next_float() * 6.0;
            if distance <= threshold || f64::from(random.next_float()) < 0.031 {
                self.try_place(world, random, p)?;
            }
        }
        Ok(true)
    }
    fn try_place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        p: [i32; 3],
    ) -> Result<(), BlockPileError<W::Error>> {
        if !self
            .air
            .contains(&world.block(p).map_err(BlockPileError::World)?)
        {
            return Ok(());
        }
        let below = [p[0], p[1].wrapping_sub(1), p[2]];
        let support = world.block(below).map_err(BlockPileError::World)?;
        let allowed = if self.paths.contains(&support) {
            random.next_boolean()
        } else {
            world
                .state_face_sturdy(support, below, Direction::Up)
                .map_err(BlockPileError::Query)?
        };
        if allowed {
            let state = self
                .provider
                .get_state(world, random, p)
                .map_err(BlockPileError::Provider)?;
            world
                .set_block(p, state, 260)
                .map_err(BlockPileError::World)?;
        }
        Ok(())
    }
}
