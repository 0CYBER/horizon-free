//! Randomized tapered spikes, mirrored lower layers and discontinuous roots.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    block_predicate::BlockPredicate,
    feature::FeatureWorld,
    placement::{PlacementContext, PlacementQueryError},
    required,
};
use crate::registry::BlockStateId;
use horizon_core::random::{InvalidBound, RandomSource};
#[derive(Clone, Debug)]
pub struct SpikeFeature {
    state: BlockStateId,
    can_place_on: BlockPredicate,
    can_replace: BlockPredicate,
    air: [BlockStateId; 3],
}
#[derive(Debug)]
pub enum SpikeError<E> {
    World(E),
    Query(PlacementQueryError),
    Random(InvalidBound),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for SpikeError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("spike work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for SpikeError<E> {}
impl SpikeFeature {
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
            return Err(DecodeError::Field("spike depth or work"));
        }
        *remaining -= 1;
        let object: Object = serde_json::from_str(json)?;
        Ok(Self {
            state: bindings
                .codec
                .decode(required(&object, "state")?.get(), false)
                .map_err(|e| DecodeError::Registry(e.to_string()))?,
            can_place_on: BlockPredicate::decode_with_blocks(
                required(&object, "can_place_on")?.get(),
                depth + 1,
                remaining,
                Some(bindings),
            )?,
            can_replace: BlockPredicate::decode_with_blocks(
                required(&object, "can_replace")?.get(),
                depth + 1,
                remaining,
                Some(bindings),
            )?,
            air: bindings.air,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        mut origin: [i32; 3],
    ) -> Result<bool, SpikeError<W::Error>> {
        let mut budget = 1_048_576;
        loop {
            spend(&mut budget)?;
            if !self
                .air
                .contains(&world.block(origin).map_err(SpikeError::World)?)
            {
                break;
            }
            if origin[1] <= world.min_y().map_err(SpikeError::Query)?.wrapping_add(2) {
                break;
            }
            origin[1] = origin[1].wrapping_sub(1);
        }
        if !self
            .can_place_on
            .test(world, origin)
            .map_err(SpikeError::Query)?
        {
            return Ok(false);
        }
        origin[1] = origin[1].wrapping_add(random.next_int_bound(4).map_err(SpikeError::Random)?);
        let height = random.next_int_bound(4).map_err(SpikeError::Random)? + 7;
        let base = height / 4 + random.next_int_bound(2).map_err(SpikeError::Random)?;
        if base > 1 && random.next_int_bound(60).map_err(SpikeError::Random)? == 0 {
            origin[1] =
                origin[1].wrapping_add(10 + random.next_int_bound(30).map_err(SpikeError::Random)?);
        }
        for layer in 0..height {
            let radius = (1.0 - layer as f32 / height as f32) * base as f32;
            let ceil = radius.ceil() as i32;
            for x in -ceil..=ceil {
                let fx = x.abs() as f32 - 0.25;
                for z in -ceil..=ceil {
                    let fz = z.abs() as f32 - 0.25;
                    if (x != 0 || z != 0) && fx * fx + fz * fz > radius * radius {
                        continue;
                    }
                    if (x == -ceil || x == ceil || z == -ceil || z == ceil)
                        && random.next_float() > 0.75
                    {
                        continue;
                    }
                    self.place_part(world, offset(origin, [x, layer, z]))?;
                    if layer != 0 && ceil > 1 {
                        self.place_part(world, offset(origin, [x, -layer, z]))?;
                    }
                }
            }
        }
        let roots = (base - 1).clamp(0, 1);
        for x in -roots..=roots {
            for z in -roots..=roots {
                let mut p = offset(origin, [x, -1, z]);
                let mut remaining = 50;
                if x.abs() == 1 && z.abs() == 1 {
                    remaining = random.next_int_bound(5).map_err(SpikeError::Random)?;
                }
                while p[1] > 50 {
                    spend(&mut budget)?;
                    let state = world.block(p).map_err(SpikeError::World)?;
                    if !self.air.contains(&state)
                        && !self.can_replace.test(world, p).map_err(SpikeError::Query)?
                        && state != self.state
                    {
                        break;
                    }
                    world
                        .set_block(p, self.state, 3)
                        .map_err(SpikeError::World)?;
                    p[1] = p[1].wrapping_sub(1);
                    remaining -= 1;
                    if remaining <= 0 {
                        p[1] = p[1].wrapping_sub(
                            random.next_int_bound(5).map_err(SpikeError::Random)? + 1,
                        );
                        remaining = random.next_int_bound(5).map_err(SpikeError::Random)?;
                    }
                }
            }
        }
        Ok(true)
    }
    fn place_part<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> Result<(), SpikeError<W::Error>> {
        if self
            .air
            .contains(&world.block(p).map_err(SpikeError::World)?)
            || self.can_replace.test(world, p).map_err(SpikeError::Query)?
        {
            world
                .set_block(p, self.state, 3)
                .map_err(SpikeError::World)?;
        }
        Ok(())
    }
}
fn offset(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| p[i].wrapping_add(d[i]))
}
fn spend<E>(remaining: &mut usize) -> Result<(), SpikeError<E>> {
    if *remaining == 0 {
        return Err(SpikeError::WorkLimit);
    }
    *remaining -= 1;
    Ok(())
}
