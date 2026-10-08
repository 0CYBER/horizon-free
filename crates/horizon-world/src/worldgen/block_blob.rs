//! Descending support search and three overlapping randomized block blobs.
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
pub struct BlockBlobFeature {
    state: BlockStateId,
    can_place_on: BlockPredicate,
}
#[derive(Debug)]
pub enum BlockBlobError<E> {
    Random(InvalidBound),
    Query(PlacementQueryError),
    World(E),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for BlockBlobError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Random(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("block blob work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BlockBlobError<E> {}
impl BlockBlobFeature {
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
            return Err(DecodeError::Field("block blob depth or work"));
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
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        mut origin: [i32; 3],
    ) -> Result<bool, BlockBlobError<W::Error>> {
        let mut remaining = 1_048_576;
        while origin[1]
            > world
                .min_y()
                .map_err(BlockBlobError::Query)?
                .wrapping_add(3)
        {
            spend(&mut remaining)?;
            let below = [origin[0], origin[1].wrapping_sub(1), origin[2]];
            if self
                .can_place_on
                .test(world, below)
                .map_err(BlockBlobError::Query)?
            {
                break;
            }
            origin = below;
        }
        if origin[1]
            <= world
                .min_y()
                .map_err(BlockBlobError::Query)?
                .wrapping_add(3)
        {
            return Ok(false);
        }
        for _ in 0..3 {
            let rx = random.next_int_bound(2).map_err(BlockBlobError::Random)?;
            let ry = random.next_int_bound(2).map_err(BlockBlobError::Random)?;
            let rz = random.next_int_bound(2).map_err(BlockBlobError::Random)?;
            let radius = (rx + ry + rz) as f32 * 0.333 + 0.5;
            let offset = [rx, ry, rz];
            let a: [i32; 3] = std::array::from_fn(|i| origin[i].wrapping_sub(offset[i]));
            let b: [i32; 3] = std::array::from_fn(|i| origin[i].wrapping_add(offset[i]));
            let min: [i32; 3] = std::array::from_fn(|i| a[i].min(b[i]));
            let sizes: [i32; 3] =
                std::array::from_fn(|i| a[i].max(b[i]).wrapping_sub(min[i]).wrapping_add(1));
            let count = sizes[0].wrapping_mul(sizes[1]).wrapping_mul(sizes[2]);
            let mut index: i32 = 0;
            while index != count {
                spend(&mut remaining)?;
                let quotient = index / sizes[0];
                let p = [
                    min[0].wrapping_add(index % sizes[0]),
                    min[1].wrapping_add(quotient % sizes[1]),
                    min[2].wrapping_add(quotient / sizes[1]),
                ];
                index = index.wrapping_add(1);
                let d: [f64; 3] = std::array::from_fn(|i| f64::from(p[i]) - f64::from(origin[i]));
                if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= f64::from(radius * radius) {
                    world
                        .set_block(p, self.state, 3)
                        .map_err(BlockBlobError::World)?;
                }
            }
            origin[0] = origin[0]
                .wrapping_add(-1 + random.next_int_bound(2).map_err(BlockBlobError::Random)?);
            origin[1] =
                origin[1].wrapping_sub(random.next_int_bound(2).map_err(BlockBlobError::Random)?);
            origin[2] = origin[2]
                .wrapping_add(-1 + random.next_int_bound(2).map_err(BlockBlobError::Random)?);
        }
        Ok(true)
    }
}
fn spend<E>(remaining: &mut usize) -> Result<(), BlockBlobError<E>> {
    if *remaining == 0 {
        return Err(BlockBlobError::WorkLimit);
    }
    *remaining -= 1;
    Ok(())
}
