//! A controlled opaque-cube scene for exercising the native renderer. This is
//! not the vanilla world generator or an authoritative gameplay simulation.
use crate::{
    Result,
    assets::{Assets, Properties},
};
use horizon_core::position::ChunkPos;
use horizon_world::{registry::BlockStateId, worldgen::terrain::TerrainChunk};
use std::collections::BTreeMap;

pub struct TestWorld<'a> {
    pub chunks: BTreeMap<(i32, i32), TerrainChunk<'a, Properties>>,
    assets: &'a Assets,
    air: BlockStateId,
}
impl<'a> TestWorld<'a> {
    pub fn new(assets: &'a Assets, radius: i32) -> Result<Self> {
        let air = assets
            .registry
            .default_state("minecraft:air")
            .ok_or("missing air state")?;
        let mut world = Self {
            chunks: BTreeMap::new(),
            assets,
            air,
        };
        for cz in -radius..=radius {
            for cx in -radius..=radius {
                let mut chunk = TerrainChunk::new(
                    ChunkPos::new(cx, cz),
                    0,
                    16,
                    &assets.registry,
                    assets.properties.clone(),
                )?;
                for z in 0..16 {
                    for x in 0..16 {
                        for y in 0..4 {
                            let layer = match y {
                                0 => 0,
                                1 | 2 => 1,
                                _ => {
                                    if (cx + cz) & 1 == 0 {
                                        2
                                    } else {
                                        3
                                    }
                                }
                            };
                            chunk.set_block_worldgen(
                                cx * 16 + x,
                                y,
                                cz * 16 + z,
                                assets.material_states[layer],
                            )?;
                        }
                    }
                }
                world.chunks.insert((cx, cz), chunk);
            }
        }
        for y in 4..11 {
            for z in -10_i32..=10 {
                for x in -10_i32..=10 {
                    if x.abs().max(z.abs()) <= 10 - (y - 4) {
                        world.set(
                            x,
                            y,
                            z,
                            assets.material_states[if y & 1 == 0 { 4 } else { 5 }],
                        )?;
                    }
                }
            }
        }
        Ok(world)
    }
    fn set(&mut self, x: i32, y: i32, z: i32, state: BlockStateId) -> Result<()> {
        self.chunks
            .get_mut(&(x >> 4, z >> 4))
            .ok_or("test scene exceeds resident chunks")?
            .set_block_worldgen(x, y, z, state)?;
        Ok(())
    }
    pub fn layer(&self, x: i32, y: i32, z: i32) -> Result<i32> {
        let state = self
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or(self.air, |chunk| chunk.block(x, y, z));
        let layer = self.assets.layers[state.0 as usize];
        if layer == -2 {
            return Err("test scene contains an unsupported non-cube material".into());
        }
        Ok(layer)
    }
}
