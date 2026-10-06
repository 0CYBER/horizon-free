//! Nether delta patches with ordered rims, shifted contents and cavity checks.
use super::{
    DecodeError, Object, block_feature::FeatureBlockBindings, feature::FeatureWorld, required,
    value_provider::IntProvider,
};
use crate::registry::BlockStateId;
use horizon_core::{
    geometry::Direction,
    random::{InvalidBound, RandomSource},
};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct DeltaFeature {
    contents: BlockStateId,
    rim: BlockStateId,
    size: IntProvider,
    rim_size: IntProvider,
    blocked: Arc<BTreeSet<BlockStateId>>,
    air: [BlockStateId; 3],
}
#[derive(Debug)]
pub enum DeltaError<E> {
    World(E),
    Random(InvalidBound),
}
impl<E: std::fmt::Display> std::fmt::Display for DeltaError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for DeltaError<E> {}
impl DeltaFeature {
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
            return Err(DecodeError::Field("delta depth or work"));
        }
        *remaining -= 1;
        let object: Object = serde_json::from_str(json)?;
        let state = |name| {
            required(&object, name).and_then(|v| {
                bindings
                    .codec
                    .decode(v.get(), false)
                    .map_err(|e| DecodeError::Registry(e.to_string()))
            })
        };
        let contents = state("contents")?;
        let rim = state("rim")?;
        let mut provider = |name| {
            let p = IntProvider::decode(required(&object, name)?, depth + 1, remaining)?;
            let [min, max] = p.bounds();
            if min < 0 || max > 16 {
                return Err(DecodeError::Field("delta size bounds"));
            }
            Ok(p)
        };
        let size = provider("size")?;
        let rim_size = provider("rim_size")?;
        let contents_name = &bindings
            .registry()
            .state(contents)
            .ok_or(DecodeError::Field("delta contents"))?
            .name;
        let mut blocked = BTreeSet::new();
        for name in [
            "minecraft:bedrock",
            "minecraft:nether_bricks",
            "minecraft:nether_brick_fence",
            "minecraft:nether_brick_stairs",
            "minecraft:nether_wart",
            "minecraft:chest",
            "minecraft:spawner",
            contents_name.as_str(),
        ] {
            blocked.extend(
                bindings
                    .blocks
                    .get(name)
                    .ok_or_else(|| DecodeError::Registry(format!("unknown delta block {name}")))?
                    .iter()
                    .copied(),
            );
        }
        Ok(Self {
            contents,
            rim,
            size,
            rim_size,
            blocked: Arc::new(blocked),
            air: bindings.air,
        })
    }
    pub fn place<W: FeatureWorld>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, DeltaError<W::Error>> {
        let draw_rim = random.next_double() < 0.9;
        let rim_x = if draw_rim {
            self.rim_size.sample(random).map_err(DeltaError::Random)?
        } else {
            0
        };
        let rim_z = if draw_rim {
            self.rim_size.sample(random).map_err(DeltaError::Random)?
        } else {
            0
        };
        let rim = draw_rim && rim_x != 0 && rim_z != 0;
        let rx = self.size.sample(random).map_err(DeltaError::Random)?;
        let rz = self.size.sample(random).map_err(DeltaError::Random)?;
        let mut any = false;
        for depth in 0..=rx.max(rz) {
            let max_x = rx.min(depth);
            for x in -max_x..=max_x {
                let z = depth - x.abs();
                if z > rz {
                    continue;
                }
                self.place_at(
                    world,
                    offset(origin, [x, 0, z]),
                    [rim_x, 0, rim_z],
                    rim,
                    &mut any,
                )?;
                if z != 0 {
                    self.place_at(
                        world,
                        offset(origin, [x, 0, -z]),
                        [rim_x, 0, rim_z],
                        rim,
                        &mut any,
                    )?;
                }
            }
        }
        Ok(any)
    }
    fn place_at<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
        shift: [i32; 3],
        rim: bool,
        any: &mut bool,
    ) -> Result<(), DeltaError<W::Error>> {
        if !self.is_clear(world, p)? {
            return Ok(());
        }
        if rim {
            world.set_block(p, self.rim, 3).map_err(DeltaError::World)?;
            *any = true;
        }
        let shifted = offset(p, shift);
        if self.is_clear(world, shifted)? {
            world
                .set_block(shifted, self.contents, 3)
                .map_err(DeltaError::World)?;
            *any = true;
        }
        Ok(())
    }
    fn is_clear<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> Result<bool, DeltaError<W::Error>> {
        if self
            .blocked
            .contains(&world.block(p).map_err(DeltaError::World)?)
        {
            return Ok(false);
        }
        for direction in Direction::ALL {
            let air = self.air.contains(
                &world
                    .block(offset(p, direction.normal()))
                    .map_err(DeltaError::World)?,
            );
            if air != (direction == Direction::Up) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
fn offset(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| p[i].wrapping_add(d[i]))
}
