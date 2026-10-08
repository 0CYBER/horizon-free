//! Packed-ice seeding and 200 ordered blue-ice growth attempts.
use super::{
    DecodeError,
    block_feature::FeatureBlockBindings,
    block_predicate::PredicateBlockResolver,
    feature::FeatureWorld,
    placement::{PlacementContext, PlacementQueryError},
};
use crate::registry::BlockStateId;
use horizon_core::{
    geometry::Direction,
    random::{InvalidBound, RandomSource},
};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct BlueIceFeature {
    water: Arc<BTreeSet<BlockStateId>>,
    packed: Arc<BTreeSet<BlockStateId>>,
    blue: Arc<BTreeSet<BlockStateId>>,
    replaceable: BTreeSet<BlockStateId>,
    output: BlockStateId,
}
#[derive(Debug)]
pub enum BlueIceError<E> {
    World(E),
    Query(PlacementQueryError),
    Random(InvalidBound),
}
impl<E: std::fmt::Display> std::fmt::Display for BlueIceError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BlueIceError<E> {}
impl BlueIceFeature {
    pub fn new(bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let water = bindings.predicate_block_set(r#""minecraft:water""#)?;
        let packed = bindings.predicate_block_set(r#""minecraft:packed_ice""#)?;
        let blue = bindings.predicate_block_set(r#""minecraft:blue_ice""#)?;
        let mut replaceable: BTreeSet<_> = bindings.air.into_iter().collect();
        replaceable.extend(water.iter().copied());
        replaceable.extend(packed.iter().copied());
        replaceable.extend(
            bindings
                .predicate_block_set(r#""minecraft:ice""#)?
                .iter()
                .copied(),
        );
        Ok(Self {
            water,
            packed,
            blue,
            replaceable,
            output: bindings
                .registry()
                .default_state("minecraft:blue_ice")
                .ok_or(DecodeError::Field("blue ice block"))?,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, BlueIceError<W::Error>> {
        if origin[1]
            > world
                .sea_level()
                .map_err(BlueIceError::Query)?
                .wrapping_sub(1)
        {
            return Ok(false);
        }
        if !self
            .water
            .contains(&world.block(origin).map_err(BlueIceError::World)?)
            && !self.water.contains(
                &world
                    .block([origin[0], origin[1].wrapping_sub(1), origin[2]])
                    .map_err(BlueIceError::World)?,
            )
        {
            return Ok(false);
        }
        let mut nearby = false;
        for direction in Direction::ALL {
            if direction == Direction::Down {
                continue;
            }
            let at = offset(origin, direction.normal());
            if self
                .packed
                .contains(&world.block(at).map_err(BlueIceError::World)?)
            {
                nearby = true;
                break;
            }
        }
        if !nearby {
            return Ok(false);
        }
        world
            .set_block(origin, self.output, 2)
            .map_err(BlueIceError::World)?;
        for _ in 0..200 {
            let dy = random.next_int_bound(5).map_err(BlueIceError::Random)?
                - random.next_int_bound(6).map_err(BlueIceError::Random)?;
            let mut radius = 3;
            if dy < 2 {
                radius += dy / 2;
            }
            if radius < 1 {
                continue;
            }
            let dx = random
                .next_int_bound(radius)
                .map_err(BlueIceError::Random)?
                - random
                    .next_int_bound(radius)
                    .map_err(BlueIceError::Random)?;
            let dz = random
                .next_int_bound(radius)
                .map_err(BlueIceError::Random)?
                - random
                    .next_int_bound(radius)
                    .map_err(BlueIceError::Random)?;
            let p = offset(origin, [dx, dy, dz]);
            if !self
                .replaceable
                .contains(&world.block(p).map_err(BlueIceError::World)?)
            {
                continue;
            }
            for direction in Direction::ALL {
                if self.blue.contains(
                    &world
                        .block(offset(p, direction.normal()))
                        .map_err(BlueIceError::World)?,
                ) {
                    world
                        .set_block(p, self.output, 2)
                        .map_err(BlueIceError::World)?;
                    break;
                }
            }
        }
        Ok(true)
    }
}
fn offset(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| p[i].wrapping_add(d[i]))
}
