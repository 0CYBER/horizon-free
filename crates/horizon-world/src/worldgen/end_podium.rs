//! End exit podium construction and active-portal replacement requests.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    optional,
    placement::{PlacementContext, PlacementQueryError},
};
use crate::registry::BlockStateId;
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct EndPodiumFeature {
    active: bool,
    bedrock: BlockStateId,
    air: BlockStateId,
    end_stone: BlockStateId,
    portal: BlockStateId,
    torches: [BlockStateId; 4],
    same_blocks: [Arc<BTreeSet<BlockStateId>>; 3],
}
#[derive(Debug)]
pub enum EndPodiumError<E> {
    World(E),
    Query(PlacementQueryError),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for EndPodiumError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("end podium work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for EndPodiumError<E> {}
impl EndPodiumFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let active = optional(&object, "active")
            .map(|v| serde_json::from_str(v.get()))
            .transpose()?
            .unwrap_or(false);
        let get = |name| {
            bindings
                .registry()
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("unknown podium block {name}")))
        };
        let torches = ["north", "east", "south", "west"].map(|direction| {
            bindings
                .codec
                .decode(
                    &format!(r#"{{"id":"wall_torch","properties":{{"facing":"{direction}"}}}}"#),
                    false,
                )
                .map_err(|e| DecodeError::Registry(e.to_string()))
        });
        let [north, east, south, west] = torches;
        let same_blocks = [
            "minecraft:end_stone",
            "minecraft:air",
            "minecraft:end_portal",
        ]
        .map(|name| {
            bindings
                .blocks
                .get(name)
                .cloned()
                .ok_or_else(|| DecodeError::Registry(format!("unknown podium block {name}")))
        });
        let [end_stone, air, portal] = same_blocks;
        Ok(Self {
            active,
            bedrock: get("minecraft:bedrock")?,
            air: get("minecraft:air")?,
            end_stone: get("minecraft:end_stone")?,
            portal: get("minecraft:end_portal")?,
            torches: [north?, east?, south?, west?],
            same_blocks: [end_stone?, air?, portal?],
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        origin: [i32; 3],
    ) -> Result<bool, EndPodiumError<W::Error>> {
        let a = offset(origin, [-4, -1, -4]);
        let b = offset(origin, [4, 32, 4]);
        let min: [i32; 3] = std::array::from_fn(|i| a[i].min(b[i]));
        let sizes: [i32; 3] =
            std::array::from_fn(|i| a[i].max(b[i]).wrapping_sub(min[i]).wrapping_add(1));
        let count = sizes[0].wrapping_mul(sizes[1]).wrapping_mul(sizes[2]);
        let mut index: i32 = 0;
        let mut remaining = 1_048_576;
        while index != count {
            if remaining == 0 {
                return Err(EndPodiumError::WorkLimit);
            }
            remaining -= 1;
            let quotient = index / sizes[0];
            let p = [
                min[0].wrapping_add(index % sizes[0]),
                min[1].wrapping_add(quotient % sizes[1]),
                min[2].wrapping_add(quotient / sizes[1]),
            ];
            index = index.wrapping_add(1);
            let d: [f64; 3] = std::array::from_fn(|i| f64::from(p[i]) - f64::from(origin[i]));
            let distance = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            let inner = distance < 2.5 * 2.5;
            if !inner && distance >= 3.5 * 3.5 {
                continue;
            }
            if p[1] < origin[1] {
                if inner {
                    world
                        .set_block(p, self.bedrock, 3)
                        .map_err(EndPodiumError::World)?;
                } else {
                    self.replace(world, p, self.end_stone, 0)?;
                }
            } else if p[1] > origin[1] {
                self.replace(world, p, self.air, 1)?;
            } else if !inner {
                world
                    .set_block(p, self.bedrock, 3)
                    .map_err(EndPodiumError::World)?;
            } else if self.active {
                self.replace(world, p, self.portal, 2)?;
            } else {
                world
                    .set_block(p, self.air, 3)
                    .map_err(EndPodiumError::World)?;
            }
        }
        for y in 0..4 {
            world
                .set_block(offset(origin, [0, y, 0]), self.bedrock, 3)
                .map_err(EndPodiumError::World)?;
        }
        for (delta, state) in [[0, 2, -1], [1, 2, 0], [0, 2, 1], [-1, 2, 0]]
            .into_iter()
            .zip(self.torches)
        {
            world
                .set_block(offset(origin, delta), state, 3)
                .map_err(EndPodiumError::World)?;
        }
        Ok(true)
    }
    fn replace<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        p: [i32; 3],
        state: BlockStateId,
        kind: usize,
    ) -> Result<(), EndPodiumError<W::Error>> {
        if self.active {
            if self.same_blocks[kind].contains(&world.block(p).map_err(EndPodiumError::World)?) {
                return Ok(());
            }
            world
                .destroy_block_without_entity(p, true)
                .map_err(EndPodiumError::Query)?;
        }
        world
            .set_block(p, state, 3)
            .map_err(EndPodiumError::World)?;
        Ok(())
    }
}
fn offset(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| p[i].wrapping_add(d[i]))
}
