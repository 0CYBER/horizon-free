//! Bamboo stalks, podzol patches and ordered cap overwrites.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    number_text,
    placement::{PlacementContext, PlacementQueryError},
    required,
};
use crate::{heightmap::HeightmapType, registry::BlockStateId};
use horizon_core::random::{InvalidBound, RandomSource};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct BambooFeature {
    probability: f32,
    air: [BlockStateId; 3],
    bamboo: BlockStateId,
    trunk: BlockStateId,
    caps: [BlockStateId; 3],
    podzol: BlockStateId,
    replaceable: Arc<BTreeSet<BlockStateId>>,
}
#[derive(Debug)]
pub enum BambooError<E> {
    World(E),
    Query(PlacementQueryError),
    Random(InvalidBound),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for BambooError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("bamboo work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BambooError<E> {}
impl BambooFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let probability = number_text(required(&object, "probability")?)?
            .parse::<f64>()
            .map_err(|_| DecodeError::Field("bamboo probability"))?
            as f32;
        if !(0.0..=1.0).contains(&probability) || probability.is_sign_negative() {
            return Err(DecodeError::Field("bamboo probability"));
        }
        let state = |leaves, stage| {
            bindings.codec.decode(&format!(r#"{{"id":"bamboo","properties":{{"age":"1","leaves":"{leaves}","stage":"{stage}"}}}}"#),false).map_err(|e|DecodeError::Registry(e.to_string()))
        };
        let get = |name| {
            bindings
                .registry()
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("unknown bamboo block {name}")))
        };
        Ok(Self {
            probability,
            air: bindings.air,
            bamboo: get("minecraft:bamboo")?,
            trunk: state("none", 0)?,
            caps: [state("large", 1)?, state("large", 0)?, state("small", 0)?],
            podzol: get("minecraft:podzol")?,
            replaceable: bindings
                .tags
                .get("minecraft:beneath_bamboo_podzol_replaceable")
                .cloned()
                .unwrap_or_default(),
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, BambooError<W::Error>> {
        if !self
            .air
            .contains(&world.block(origin).map_err(BambooError::World)?)
        {
            return Ok(false);
        }
        // Native success records the air-origin attempt, even if survival fails.
        if !world
            .block_survives(self.bamboo, origin)
            .map_err(BambooError::Query)?
        {
            return Ok(true);
        }
        let height = random.next_int_bound(12).map_err(BambooError::Random)? + 5;
        if random.next_float() < self.probability {
            let radius = random.next_int_bound(4).map_err(BambooError::Random)? + 1;
            let mut x = origin[0].wrapping_sub(radius);
            let max_x = origin[0].wrapping_add(radius);
            let mut remaining = 1_048_576;
            while x <= max_x {
                let mut z = origin[2].wrapping_sub(radius);
                let max_z = origin[2].wrapping_add(radius);
                while z <= max_z {
                    if remaining == 0 {
                        return Err(BambooError::WorkLimit);
                    }
                    remaining -= 1;
                    let dx = x.wrapping_sub(origin[0]);
                    let dz = z.wrapping_sub(origin[2]);
                    if dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) <= radius * radius {
                        let y = world
                            .height(HeightmapType::WorldSurface, x, z)
                            .map_err(BambooError::Query)?
                            .wrapping_sub(1);
                        let p = [x, y, z];
                        if self
                            .replaceable
                            .contains(&world.block(p).map_err(BambooError::World)?)
                        {
                            world
                                .set_block(p, self.podzol, 2)
                                .map_err(BambooError::World)?;
                        }
                    }
                    z = z.wrapping_add(1);
                }
                // Also bound overflow loops when the inner range is empty.
                if remaining == 0 {
                    return Err(BambooError::WorkLimit);
                }
                remaining -= 1;
                x = x.wrapping_add(1);
            }
        }
        let mut p = origin;
        for _ in 0..height {
            if !self
                .air
                .contains(&world.block(p).map_err(BambooError::World)?)
            {
                break;
            }
            world
                .set_block(p, self.trunk, 2)
                .map_err(BambooError::World)?;
            p[1] = p[1].wrapping_add(1);
        }
        if p[1].wrapping_sub(origin[1]) >= 3 {
            // The first cap occupies the stopping position, including an obstacle.
            for state in self.caps {
                world.set_block(p, state, 2).map_err(BambooError::World)?;
                p[1] = p[1].wrapping_sub(1);
            }
        }
        Ok(true)
    }
}
