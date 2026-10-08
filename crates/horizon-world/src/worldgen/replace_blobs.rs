//! Block-identity replacement in clipped Manhattan shells around a ground target.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    placement::{PlacementContext, PlacementQueryError},
    required,
    value_provider::IntProvider,
};
use crate::registry::BlockStateId;
use horizon_core::random::{InvalidBound, RandomSource};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct ReplaceBlobsFeature {
    targets: Arc<BTreeSet<BlockStateId>>,
    state: BlockStateId,
    radius: IntProvider,
}
#[derive(Debug)]
pub enum ReplaceBlobsError<E> {
    World(E),
    Query(PlacementQueryError),
    Random(InvalidBound),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for ReplaceBlobsError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("replace blobs work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for ReplaceBlobsError<E> {}
impl ReplaceBlobsFeature {
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
            return Err(DecodeError::Field("replace blobs depth or work"));
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
        let target = state("target")?;
        let name = &bindings
            .registry()
            .state(target)
            .ok_or(DecodeError::Field("replace blobs target"))?
            .name;
        let targets = bindings
            .blocks
            .get(name)
            .cloned()
            .ok_or(DecodeError::Field("replace blobs target block"))?;
        let radius = IntProvider::decode(required(&object, "radius")?, depth + 1, remaining)?;
        let [min, max] = radius.bounds();
        if min < 0 || max > 12 {
            return Err(DecodeError::Field("replace blobs radius bounds"));
        }
        Ok(Self {
            targets,
            state: state("state")?,
            radius,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        mut origin: [i32; 3],
    ) -> Result<bool, ReplaceBlobsError<W::Error>> {
        let min = world
            .min_y()
            .map_err(ReplaceBlobsError::Query)?
            .wrapping_add(1);
        let max = world.max_y().map_err(ReplaceBlobsError::Query)?;
        // Match Mth.clamp, including inverted world bounds.
        origin[1] = origin[1].max(min).min(max);
        let mut remaining = 1_048_576;
        loop {
            if origin[1]
                <= world
                    .min_y()
                    .map_err(ReplaceBlobsError::Query)?
                    .wrapping_add(1)
            {
                return Ok(false);
            }
            if remaining == 0 {
                return Err(ReplaceBlobsError::WorkLimit);
            }
            remaining -= 1;
            if self
                .targets
                .contains(&world.block(origin).map_err(ReplaceBlobsError::World)?)
            {
                break;
            }
            origin[1] = origin[1].wrapping_sub(1);
        }
        let rx = self
            .radius
            .sample(random)
            .map_err(ReplaceBlobsError::Random)?;
        let ry = self
            .radius
            .sample(random)
            .map_err(ReplaceBlobsError::Random)?;
        let rz = self
            .radius
            .sample(random)
            .map_err(ReplaceBlobsError::Random)?;
        let mut any = false;
        // Native order is depth, ascending X, ascending Y, positive Z then its
        // mirror. The feature stops before the first shell beyond max radius.
        for depth in 0..=rx.max(ry).max(rz) {
            let max_x = rx.min(depth);
            for x in -max_x..=max_x {
                let max_y = ry.min(depth - x.abs());
                for y in -max_y..=max_y {
                    let z = depth - x.abs() - y.abs();
                    if z > rz {
                        continue;
                    }
                    self.replace(world, offset(origin, [x, y, z]), &mut any)?;
                    if z != 0 {
                        self.replace(world, offset(origin, [x, y, -z]), &mut any)?;
                    }
                }
            }
        }
        Ok(any)
    }
    fn replace<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
        any: &mut bool,
    ) -> Result<(), ReplaceBlobsError<W::Error>> {
        if self
            .targets
            .contains(&world.block(p).map_err(ReplaceBlobsError::World)?)
        {
            world
                .set_block(p, self.state, 3)
                .map_err(ReplaceBlobsError::World)?;
            *any = true;
        }
        Ok(())
    }
}
fn offset(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| p[i].wrapping_add(d[i]))
}
