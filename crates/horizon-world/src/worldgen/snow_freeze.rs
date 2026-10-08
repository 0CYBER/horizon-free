//! Freeze exposed water and place snow across a 16-by-16 feature footprint.
use super::{
    DecodeError,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    placement::{PlacementContext, PlacementQueryError},
};
use crate::{heightmap::HeightmapType, registry::BlockStateId};
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Debug)]
pub struct SnowAndFreezeFeature {
    ice: BlockStateId,
    snow: BlockStateId,
    snowy: Arc<BTreeMap<BlockStateId, BlockStateId>>,
}
#[derive(Debug)]
pub enum SnowAndFreezeError<E> {
    World(E),
    Query(PlacementQueryError),
}
impl<E: std::fmt::Display> std::fmt::Display for SnowAndFreezeError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for SnowAndFreezeError<E> {}
impl SnowAndFreezeFeature {
    pub fn new(bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let registry = bindings.registry();
        let get = |name| {
            registry
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("unknown freezing block {name}")))
        };
        // These are the pinned registry's blocks with SnowyBlock.SNOWY.
        let mut snowy = BTreeMap::new();
        for name in [
            "minecraft:grass_block",
            "minecraft:podzol",
            "minecraft:mycelium",
        ] {
            for state in bindings
                .blocks
                .get(name)
                .ok_or(DecodeError::Field("snowy block"))?
                .iter()
            {
                let mut descriptor = registry
                    .state(*state)
                    .ok_or(DecodeError::Field("snowy state"))?
                    .clone();
                descriptor
                    .properties
                    .insert("snowy".to_owned(), "true".to_owned());
                snowy.insert(
                    *state,
                    registry
                        .id(&descriptor)
                        .ok_or(DecodeError::Field("snowy true state"))?,
                );
            }
        }
        Ok(Self {
            ice: get("minecraft:ice")?,
            snow: get("minecraft:snow")?,
            snowy: Arc::new(snowy),
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        origin: [i32; 3],
    ) -> Result<bool, SnowAndFreezeError<W::Error>> {
        for dx in 0..16 {
            for dz in 0..16 {
                let x = origin[0].wrapping_add(dx);
                let z = origin[2].wrapping_add(dz);
                let y = world
                    .height(HeightmapType::MotionBlocking, x, z)
                    .map_err(SnowAndFreezeError::Query)?;
                let top = [x, y, z];
                let below = [x, y.wrapping_sub(1), z];
                let biome = world
                    .biome_identity(top)
                    .map_err(SnowAndFreezeError::Query)?;
                if world
                    .biome_should_freeze(biome, below, false)
                    .map_err(SnowAndFreezeError::Query)?
                {
                    world
                        .set_block(below, self.ice, 2)
                        .map_err(SnowAndFreezeError::World)?;
                }
                if world
                    .biome_should_snow(biome, top)
                    .map_err(SnowAndFreezeError::Query)?
                {
                    world
                        .set_block(top, self.snow, 2)
                        .map_err(SnowAndFreezeError::World)?;
                    let state = world.block(below).map_err(SnowAndFreezeError::World)?;
                    if let Some(state) = self.snowy.get(&state) {
                        world
                            .set_block(below, *state, 2)
                            .map_err(SnowAndFreezeError::World)?;
                    }
                }
            }
        }
        Ok(true)
    }
}
