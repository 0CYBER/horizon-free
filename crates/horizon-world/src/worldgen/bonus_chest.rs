//! Spawn bonus chest search, loot-table assignment and surrounding torches.
use super::{
    DecodeError,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    placement::{PlacementContext, PlacementQueryError},
};
use crate::{heightmap::HeightmapType, registry::BlockStateId};
use horizon_core::random::{InvalidBound, RandomSource};
#[derive(Clone, Debug)]
pub struct BonusChestFeature {
    air: [BlockStateId; 3],
    chest: BlockStateId,
    torch: BlockStateId,
}
#[derive(Debug)]
pub enum BonusChestError<E> {
    World(E),
    Query(PlacementQueryError),
    Random(InvalidBound),
}
impl<E: std::fmt::Display> std::fmt::Display for BonusChestError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BonusChestError<E> {}
impl BonusChestFeature {
    pub fn new(bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let get = |name| {
            bindings
                .registry()
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("unknown bonus chest block {name}")))
        };
        Ok(Self {
            air: bindings.air,
            chest: get("minecraft:chest")?,
            torch: get("minecraft:torch")?,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, BonusChestError<W::Error>> {
        let mut xs: [i32; 16] = std::array::from_fn(|i| (origin[0] & !15).wrapping_add(i as i32));
        let mut zs: [i32; 16] = std::array::from_fn(|i| (origin[2] & !15).wrapping_add(i as i32));
        for values in [&mut xs, &mut zs] {
            for end in (2..=16).rev() {
                let other = random
                    .next_int_bound(end as i32)
                    .map_err(BonusChestError::Random)? as usize;
                values.swap(end - 1, other);
            }
        }
        for x in xs {
            for z in zs {
                let y = world
                    .height(HeightmapType::MotionBlockingNoLeaves, x, z)
                    .map_err(BonusChestError::Query)?;
                let p = [x, y, z];
                if !self
                    .air
                    .contains(&world.block(p).map_err(BonusChestError::World)?)
                {
                    let state = world.block(p).map_err(BonusChestError::World)?;
                    if !world
                        .state_collision_empty(state, p)
                        .map_err(BonusChestError::Query)?
                    {
                        continue;
                    }
                }
                world
                    .set_block(p, self.chest, 2)
                    .map_err(BonusChestError::World)?;
                world
                    .set_block_entity_loot_table(p, "minecraft:chests/spawn_bonus_chest", random)
                    .map_err(BonusChestError::Query)?;
                for [dx, dz] in [[0, -1], [1, 0], [0, 1], [-1, 0]] {
                    let at = [x.wrapping_add(dx), y, z.wrapping_add(dz)];
                    if world
                        .block_survives(self.torch, at)
                        .map_err(BonusChestError::Query)?
                    {
                        world
                            .set_block(at, self.torch, 2)
                            .map_err(BonusChestError::World)?;
                    }
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
}
