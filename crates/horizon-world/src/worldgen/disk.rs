//! Circular descending replacement columns and postprocessing above each run.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    block_predicate::BlockPredicate,
    feature::FeatureWorld,
    int_range,
    placement::{PlacementContext, PlacementQueryError},
    required,
    state_provider::{BlockStateProvider, StateProviderError},
    value_provider::IntProvider,
};
use crate::registry::BlockStateId;
use horizon_core::random::{InvalidBound, RandomSource};
#[derive(Clone, Debug)]
pub struct DiskFeature {
    provider: BlockStateProvider,
    target: BlockPredicate,
    radius: IntProvider,
    half_height: i32,
    air: [BlockStateId; 3],
}
#[derive(Debug)]
pub enum DiskError<E> {
    Random(InvalidBound),
    Provider(StateProviderError<E>),
    Query(PlacementQueryError),
    World(E),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for DiskError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Random(e) => e.fmt(f),
            Self::Provider(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("disk feature work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for DiskError<E> {}
impl DiskFeature {
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
            return Err(DecodeError::Field("disk depth or work"));
        }
        *remaining -= 1;
        let object: Object = serde_json::from_str(json)?;
        let (providers, catalog, _) = bindings
            .simple_blocks
            .as_ref()
            .ok_or(DecodeError::Field("disk provider bindings"))?;
        let provider = catalog.provider_with_budget(
            required(&object, "state_provider")?.get(),
            providers,
            depth + 1,
            remaining,
        )?;
        let target = BlockPredicate::decode_with_blocks(
            required(&object, "target")?.get(),
            depth + 1,
            remaining,
            Some(bindings),
        )?;
        let radius = IntProvider::decode(required(&object, "radius")?, depth + 1, remaining)?;
        if radius.bounds()[0] < 0 || radius.bounds()[1] > 8 {
            return Err(DecodeError::Field("disk radius range"));
        }
        let half_height = int_range(required(&object, "half_height")?, "disk half height", 0, 4)?;
        Ok(Self {
            provider,
            target,
            radius,
            half_height,
            air: bindings.air,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, DiskError<W::Error>> {
        let top = origin[1].wrapping_add(self.half_height);
        let bottom = origin[1].wrapping_sub(self.half_height).wrapping_sub(1);
        let radius = self.radius.sample(random).map_err(DiskError::Random)?;
        let x0 = origin[0].wrapping_sub(radius);
        let x1 = origin[0].wrapping_add(radius);
        let z0 = origin[2].wrapping_sub(radius);
        let z1 = origin[2].wrapping_add(radius);
        let min_x = x0.min(x1);
        let min_z = z0.min(z1);
        let width = x0.max(x1).wrapping_sub(min_x).wrapping_add(1);
        let depth = z0.max(z1).wrapping_sub(min_z).wrapping_add(1);
        let count = width.wrapping_mul(depth);
        let mut index = 0;
        let mut remaining = 1_048_576;
        let mut any = false;
        while index != count {
            spend(&mut remaining)?;
            let x = min_x.wrapping_add(index % width);
            let z = min_z.wrapping_add(index / width);
            index = index.wrapping_add(1);
            let dx = x.wrapping_sub(origin[0]);
            let dz = z.wrapping_sub(origin[2]);
            if dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) > radius.wrapping_mul(radius) {
                continue;
            }
            let mut y = top;
            let mut previous_placed = false;
            while y > bottom {
                spend(&mut remaining)?;
                let p = [x, y, z];
                if self.target.test(world, p).map_err(DiskError::Query)? {
                    if let Some(state) = self
                        .provider
                        .get_optional_state(world, random, p)
                        .map_err(DiskError::Provider)?
                    {
                        world.set_block(p, state, 2).map_err(DiskError::World)?;
                        if !previous_placed {
                            self.mark_above(world, p)?;
                        }
                        any = true;
                        previous_placed = true;
                    }
                } else {
                    previous_placed = false;
                }
                y = y.wrapping_sub(1);
            }
        }
        Ok(any)
    }
    fn mark_above<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        mut p: [i32; 3],
    ) -> Result<(), DiskError<W::Error>> {
        for _ in 0..2 {
            p[1] = p[1].wrapping_add(1);
            if self
                .air
                .contains(&world.block(p).map_err(DiskError::World)?)
            {
                break;
            }
            world.mark_postprocessing(p).map_err(DiskError::Query)?;
        }
        Ok(())
    }
}
fn spend<E>(remaining: &mut usize) -> Result<(), DiskError<E>> {
    if *remaining == 0 {
        return Err(DiskError::WorkLimit);
    }
    *remaining -= 1;
    Ok(())
}
