//! FlatLevelSource: superflat worlds (`minecraft:flat` generator).
use crate::{chunk::Chunk, streaming::ChunkGenerator};
use horizon_world::{nbt::Tag, registry::BlockRegistry};

/// The layers of a flat generator, bottom up from the world's minimum Y.
pub struct FlatTerrain {
    layers: Vec<u32>,
    biome: u16,
}
impl FlatTerrain {
    /// The overworld generator of `world_gen_settings` (`data.dimensions`), if it
    /// is flat. Its `features` and `lakes` decorate the world; a world without
    /// them (the default) gets only its layers, as FlatLevelSource places them.
    pub fn from_settings(
        settings: &Tag,
        registry: &BlockRegistry,
        biomes: &crate::biome::Biomes,
    ) -> Option<Self> {
        let generator = settings
            .get("dimensions")?
            .get("minecraft:overworld")?
            .get("generator")?;
        if crate::save::text(generator.get("type"))?.trim_start_matches("minecraft:") != "flat" {
            return None;
        }
        let s = generator.get("settings")?;
        let biome = crate::save::text(s.get("biome")).unwrap_or_else(|| "minecraft:plains".into());
        let mut layers = Vec::new();
        if let Some(Tag::List(list)) = s.get("layers") {
            for layer in list {
                let block = crate::save::text(layer.get("block"))?;
                let height = crate::save::number(layer.get("height")).unwrap_or(0).max(0);
                let state = registry.default_state(&block)?.0;
                layers.extend(std::iter::repeat_n(state, height as usize));
            }
        }
        Some(Self {
            layers,
            biome: biomes.id(&biome),
        })
    }
    /// FlatLevelSource.getSpawnHeight: the top of the layers.
    pub fn spawn_height(&self) -> i32 {
        crate::chunk::MIN_Y + self.layers.len().min(crate::chunk::HEIGHT as usize) as i32
    }
}
impl ChunkGenerator for FlatTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(&self, _: &mut (), cx: i32, cz: i32) -> crate::streaming::Proto {
        let mut chunk = Chunk::empty(cx, cz);
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).biomes = horizon_core::paletted::PalettedContainer::new(
                crate::chunk::BIOME_STRATEGY,
                u32::from(self.biome),
            );
        }
        for (i, &block) in self.layers.iter().enumerate().take(384) {
            if block == 0 {
                continue;
            }
            for z in 0..16 {
                for x in 0..16 {
                    chunk.set(x, -64 + i as i32, z, block);
                }
            }
        }
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).compact();
        }
        Box::new(chunk)
    }
    /// FlatLevelSource.spawnOriginalMobs is empty: no generation-time spawns.
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        *proto.downcast::<Chunk>().expect("flat terrain proto")
    }
}
