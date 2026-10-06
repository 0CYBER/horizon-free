//! Resident pre-light chunks shared by modifier queries and feature mutations.
//! The caller supplies world write policy/effects; storage alone cannot implement
//! neighbor updates, block entities, POIs or lighting.
use super::{
    anchor::WorldGenerationContext,
    biome_chunk::{BiomeRegion, BiomeRegionError},
    feature::FeatureWorld,
    placement::{LayerBlock, PlacementContext, PlacementQueryError},
    terrain::{PreLightChunkStage, TerrainBlockStates, TerrainChunk, TerrainError},
};
use crate::{
    heightmap::{HeightmapState, HeightmapType},
    registry::BlockStateId,
};
use horizon_core::position::BlockPos;
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum RegionError {
    InvalidChunks,
    MissingChunk([i32; 2]),
    Terrain(TerrainError),
    Biome(BiomeRegionError),
    BiomesNotGenerated([i32; 2]),
}
impl std::fmt::Display for RegionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RegionError {}
impl From<TerrainError> for RegionError {
    fn from(error: TerrainError) -> Self {
        Self::Terrain(error)
    }
}
impl From<RegionError> for PlacementQueryError {
    fn from(error: RegionError) -> Self {
        Self::Failed(error.to_string())
    }
}
pub struct TerrainRegion<'r, B: TerrainBlockStates> {
    chunks: BTreeMap<(i32, i32), TerrainChunk<'r, B>>,
    min_y: i32,
    height: i32,
    generation: WorldGenerationContext,
    sea_level: i32,
}
impl<'r, B: TerrainBlockStates> TerrainRegion<'r, B> {
    pub fn new(
        chunks: Vec<TerrainChunk<'r, B>>,
        generator: WorldGenerationContext,
    ) -> Result<Self, RegionError> {
        if chunks.is_empty() || chunks.len() > 4096 {
            return Err(RegionError::InvalidChunks);
        }
        let first = &chunks[0];
        let (min_y, height, registry) = (first.min_y(), first.height(), first.registry());
        if chunks.iter().any(|c| {
            c.min_y() != min_y || c.height() != height || !std::ptr::eq(c.registry(), registry)
        }) {
            return Err(RegionError::InvalidChunks);
        }
        let mut stored = BTreeMap::new();
        for chunk in chunks {
            let p = chunk.position();
            if stored.insert((p.x, p.z), chunk).is_some() {
                return Err(RegionError::InvalidChunks);
            }
        }
        Ok(Self {
            chunks: stored,
            sea_level: generator.sea_level,
            min_y,
            height,
            generation: WorldGenerationContext::with_generator(
                min_y,
                height,
                generator.min_y,
                generator.height,
                generator.sea_level,
            ),
        })
    }
    /// Override world sea level without changing generator placement anchors.
    pub fn with_world_sea_level(mut self, sea_level: i32) -> Self {
        self.sea_level = sea_level;
        self
    }
    pub fn chunks(&self) -> impl Iterator<Item = &TerrainChunk<'r, B>> {
        self.chunks.values()
    }
    pub fn chunk(&self, x: i32, z: i32) -> Result<&TerrainChunk<'r, B>, RegionError> {
        self.chunks
            .get(&(x, z))
            .ok_or(RegionError::MissingChunk([x, z]))
    }
    pub fn chunk_mut(&mut self, x: i32, z: i32) -> Result<&mut TerrainChunk<'r, B>, RegionError> {
        self.chunks
            .get_mut(&(x, z))
            .ok_or(RegionError::MissingChunk([x, z]))
    }
    pub fn block(&self, [x, y, z]: [i32; 3]) -> Result<BlockStateId, RegionError> {
        Ok(self.chunk(x >> 4, z >> 4)?.block(x, y, z))
    }
    /// Level-style biome lookup checks the selected chunk's generation status
    /// before reading its palette. Raw biome storage remains independently
    /// accessible for generation itself. Below-zero retrogen is not modeled.
    pub fn biome_at<'b, T: Clone>(
        &self,
        biomes: &'b BiomeRegion<T>,
        position: BlockPos,
    ) -> Result<&'b T, RegionError> {
        self.noise_biome(biomes, biomes.quart_at_block(position))
    }
    pub fn noise_biome<'b, T: Clone>(
        &self,
        biomes: &'b BiomeRegion<T>,
        quart: [i32; 3],
    ) -> Result<&'b T, RegionError> {
        let chunk = self.chunk(quart[0] >> 2, quart[2] >> 2)?;
        if !matches!(
            chunk.generation_stage(),
            PreLightChunkStage::Biomes | PreLightChunkStage::Terrain | PreLightChunkStage::Features
        ) {
            return Err(RegionError::BiomesNotGenerated([
                quart[0] >> 2,
                quart[2] >> 2,
            ]));
        }
        biomes.noise_biome(quart).map_err(RegionError::Biome)
    }
    /// Storage mutation only. World code must apply its write permissions,
    /// block-entity/POI effects and postprocessing around this operation.
    pub fn set_block_worldgen(
        &mut self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
    ) -> Result<BlockStateId, RegionError> {
        Ok(self
            .chunk_mut(x >> 4, z >> 4)?
            .set_block_worldgen(x, y, z, state)?)
    }
}
impl<B: TerrainBlockStates> PlacementContext for TerrainRegion<'_, B> {
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.sea_level)
    }

    fn mark_postprocessing(&mut self, [x, y, z]: [i32; 3]) -> Result<(), PlacementQueryError> {
        self.chunk_mut(x >> 4, z >> 4)?.mark_postprocessing(x, y, z);
        Ok(())
    }
    fn generation(&self) -> WorldGenerationContext {
        self.generation
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.block(position).map_err(Into::into)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.min_y)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.min_y.wrapping_add(self.height).wrapping_sub(1))
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        let chunk = self.chunk_mut(x >> 4, z >> 4)?;
        if chunk.heightmap(kind).is_none() {
            chunk.prime_heightmaps(&[kind]).map_err(RegionError::from)?;
        }
        chunk
            .heightmap(kind)
            .unwrap()
            .first_available(x & 15, z & 15)
            .map_err(|e| RegionError::from(TerrainError::from(e)).into())
    }
    fn layer_block(&mut self, position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        let chunk = self.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let name = &chunk
            .registry()
            .state(state)
            .expect("registered chunk state")
            .name;
        Ok(
            if chunk.block_properties(state).is_air()
                || name == "minecraft:water"
                || name == "minecraft:lava"
            {
                LayerBlock::Empty
            } else if name == "minecraft:bedrock" {
                LayerBlock::Bedrock
            } else {
                LayerBlock::Ground
            },
        )
    }
}
/// Gives feature algorithms and placement queries one region. All update flags
/// reach the supplied write function, which owns world mutation policy/effects.
pub struct TerrainFeatureAccess<'a, 'r, B: TerrainBlockStates, F> {
    region: &'a mut TerrainRegion<'r, B>,
    write: F,
}
impl<'a, 'r, B: TerrainBlockStates, F> TerrainFeatureAccess<'a, 'r, B, F> {
    pub fn new(region: &'a mut TerrainRegion<'r, B>, write: F) -> Self {
        Self { region, write }
    }
}
impl<B: TerrainBlockStates, F> PlacementContext for TerrainFeatureAccess<'_, '_, B, F> {
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.sea_level()
    }

    fn mark_postprocessing(&mut self, position: [i32; 3]) -> Result<(), PlacementQueryError> {
        self.region.mark_postprocessing(position)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.region.generation()
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.region.block_state(position)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.min_y()
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.max_y()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        self.region.height(kind, x, z)
    }
    fn layer_block(&mut self, position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        self.region.layer_block(position)
    }
}
impl<'r, B: TerrainBlockStates, F> FeatureWorld for TerrainFeatureAccess<'_, 'r, B, F>
where
    F: FnMut(&mut TerrainRegion<'r, B>, [i32; 3], BlockStateId, i32) -> Result<bool, RegionError>,
{
    type Error = RegionError;
    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        self.region.block(position)
    }
    fn set_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, Self::Error> {
        (self.write)(self.region, position, state, flags)
    }
}
