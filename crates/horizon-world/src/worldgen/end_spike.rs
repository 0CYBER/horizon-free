//! `minecraft:end_spike`: the obsidian pillars around the main End island.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    feature_graph::FeatureExecutionError,
    integer, optional,
    placement::{PlacementContext, PlacementQueryError},
    required,
    tree::{TreeResult, kind, write},
    tree_blocks::{self as tb, TreeBlocks},
};
use crate::registry::BlockStateId;
use horizon_core::random::{LegacyRandom, RandomSource};
use std::sync::Arc;

fn query<E>(error: PlacementQueryError) -> FeatureExecutionError<E> {
    FeatureExecutionError::Query(error)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndSpike {
    pub center_x: i32,
    pub center_z: i32,
    pub radius: i32,
    pub height: i32,
    pub guarded: bool,
}
/// EndSpikeFeature.getSpikesForLevel (SpikeCacheLoader), from the world seed.
pub fn spikes_for_seed(seed: i64) -> Vec<EndSpike> {
    let key = LegacyRandom::new(seed).next_long() & 65535;
    let mut random = LegacyRandom::new(key);
    let mut sizes: Vec<i32> = (0..10).collect();
    tb::shuffle(&mut sizes, &mut random).expect("positive shuffle bounds");
    (0..10)
        .map(|i| {
            let angle =
                2.0 * (-std::f64::consts::PI + (std::f64::consts::PI / 10.0) * f64::from(i));
            let size = sizes[i as usize];
            EndSpike {
                center_x: (42.0 * horizon_core::jmath::cos(angle)).floor() as i32,
                center_z: (42.0 * horizon_core::jmath::sin(angle)).floor() as i32,
                radius: 2 + size / 3,
                height: 76 + size * 3,
                guarded: size == 1 || size == 2,
            }
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct EndSpikeFeature {
    spikes: Vec<EndSpike>,
    crystal_invulnerable: bool,
    beam_target: Option<[i32; 3]>,
    obsidian: BlockStateId,
    air: BlockStateId,
    iron_bars: BlockStateId,
    bedrock: BlockStateId,
    fire: BlockStateId,
    blocks: Arc<TreeBlocks>,
}
impl EndSpikeFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:end_spike" {
            return Err(DecodeError::Field("end spike type"));
        }
        let raw: Vec<Object> = serde_json::from_str(required(&object, "spikes")?.get())?;
        let int = |o: &Object, name: &str| -> Result<i32, DecodeError> {
            optional(o, name)
                .map(integer)
                .transpose()
                .map(|v| v.unwrap_or(0))
        };
        let spikes = raw
            .iter()
            .map(|o| {
                Ok(EndSpike {
                    center_x: int(o, "centerX")?,
                    center_z: int(o, "centerZ")?,
                    radius: int(o, "radius")?,
                    height: int(o, "height")?,
                    guarded: optional(o, "guarded")
                        .map(|v| serde_json::from_str::<bool>(v.get()))
                        .transpose()?
                        .unwrap_or(false),
                })
            })
            .collect::<Result<_, DecodeError>>()?;
        let beam_target = optional(&object, "crystal_beam_target")
            .map(|raw| -> Result<[i32; 3], DecodeError> {
                let v: Vec<Box<serde_json::value::RawValue>> = serde_json::from_str(raw.get())?;
                if v.len() != 3 {
                    return Err(DecodeError::Field("beam target"));
                }
                Ok([integer(&v[0])?, integer(&v[1])?, integer(&v[2])?])
            })
            .transpose()?;
        let registry = bindings.registry();
        let default = |name: &str| {
            registry
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("missing spike block {name}")))
        };
        Ok(Self {
            spikes,
            crystal_invulnerable: optional(&object, "crystal_invulnerable")
                .map(|v| serde_json::from_str::<bool>(v.get()))
                .transpose()?
                .unwrap_or(false),
            beam_target,
            obsidian: default("minecraft:obsidian")?,
            air: default("minecraft:air")?,
            iron_bars: default("minecraft:iron_bars")?,
            bedrock: default("minecraft:bedrock")?,
            fire: default("minecraft:fire")?,
            blocks: bindings.tree_blocks()?,
        })
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let spikes = if self.spikes.is_empty() {
            spikes_for_seed(world.world_seed().map_err(query)?)
        } else {
            self.spikes.clone()
        };
        for spike in spikes {
            if origin[0] >> 4 == spike.center_x >> 4 && origin[2] >> 4 == spike.center_z >> 4 {
                self.place_spike(world, random, spike)?;
            }
        }
        Ok(true)
    }
    fn bars<E>(
        &self,
        north: bool,
        south: bool,
        west: bool,
        east: bool,
    ) -> TreeResult<BlockStateId, E> {
        let mut state = self.iron_bars;
        for (name, value) in [
            ("north", north),
            ("south", south),
            ("west", west),
            ("east", east),
        ] {
            state = self
                .blocks
                .with(state, name, if value { "true" } else { "false" })
                .ok_or(FeatureExecutionError::InvalidState)?;
        }
        Ok(state)
    }
    fn place_spike<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        spike: EndSpike,
    ) -> TreeResult<(), W::Error> {
        let r = spike.radius;
        let min_y = world.min_y().map_err(query)?;
        let (cx, cz) = (spike.center_x, spike.center_z);
        for z in cz - r..=cz + r {
            for y in min_y..=spike.height + 10 {
                for x in cx - r..=cx + r {
                    let dx = f64::from(x) - f64::from(cx);
                    let dz = f64::from(z) - f64::from(cz);
                    let p = [x, y, z];
                    if dx * dx + dz * dz <= f64::from(r * r + 1) && y < spike.height {
                        write(world, p, self.obsidian, 3)?;
                    } else if y > 65 {
                        write(world, p, self.air, 3)?;
                    }
                }
            }
        }
        if spike.guarded {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    for dy in 0..=3 {
                        let x_side = dx.abs() == 2;
                        let z_side = dz.abs() == 2;
                        let top = dy == 3;
                        if x_side || z_side || top {
                            let x_edge = dx == -2 || dx == 2 || top;
                            let z_edge = dz == -2 || dz == 2 || top;
                            let state = self.bars(
                                x_edge && dz != -2,
                                x_edge && dz != 2,
                                z_edge && dx != -2,
                                z_edge && dx != 2,
                            )?;
                            write(world, [cx + dx, spike.height + dy, cz + dz], state, 3)?;
                        }
                    }
                }
            }
        }
        let yaw = random.next_float() * 360.0;
        world
            .spawn_end_crystal(
                [
                    f64::from(cx) + 0.5,
                    f64::from(spike.height + 1),
                    f64::from(cz) + 0.5,
                ],
                yaw,
                self.beam_target,
                self.crystal_invulnerable,
            )
            .map_err(query)?;
        let crystal = [cx, spike.height + 1, cz];
        write(world, tb::offset(crystal, 0, -1, 0), self.bedrock, 3)?;
        // FireBlock.getState above sturdy bedrock is the plain fire default.
        write(world, crystal, self.fire, 3)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spike_layout_is_a_permutation_of_sizes() {
        let spikes = spikes_for_seed(0);
        assert_eq!(spikes.len(), 10);
        let mut heights: Vec<i32> = spikes.iter().map(|s| s.height).collect();
        heights.sort_unstable();
        assert_eq!(heights, (0..10).map(|s| 76 + s * 3).collect::<Vec<_>>());
        assert_eq!(spikes.iter().filter(|s| s.guarded).count(), 2);
        assert_eq!((spikes[0].center_x, spikes[0].center_z), (42, 0));
        assert_eq!(spikes_for_seed(0), spikes_for_seed(0));
    }
}
