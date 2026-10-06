//! Preliminary surface density sampling scoped to one material context.
use super::{MaterialRuleError, MaterialSurfaceQueries};
use crate::{registry::BlockStateId, worldgen::DecodeError};
use horizon_core::{
    density::{DensitySampler, SamplerContext},
    noise::{DensityVolume, Noise},
    position::BlockPos,
};
use std::{collections::BTreeSet, sync::Arc};

/// A lazy unit-spaced XZ plane and scalar fallback for preliminary heights.
/// Construct a fresh provider for each material context, with that world's
/// compiled preliminary-surface density root and bound sampler context.
pub struct MaterialDensitySources<S: MaterialSurfaceQueries> {
    surface: S,
    expected: DensityVolume,
    densities: Vec<BoundDensity>,
    retained_samples: usize,
    plane: DensityVolume,
    sampler: Arc<dyn DensitySampler>,
    context: SamplerContext,
    buffer: Option<Vec<f32>>,
}
struct BoundDensity {
    sampler: Arc<dyn DensitySampler>,
    buffer: Option<Vec<f32>>,
}
impl<S: MaterialSurfaceQueries> MaterialDensitySources<S> {
    pub fn sampler_context_mut(&mut self) -> &mut SamplerContext {
        &mut self.context
    }
    pub fn into_parts(self) -> (S, SamplerContext) {
        (self.surface, self.context)
    }
    pub fn new(
        surface: S,
        expected: DensityVolume,
        sampler: Arc<dyn DensitySampler>,
        context: SamplerContext,
    ) -> Result<Self, DecodeError> {
        let [sx, _, sz] = expected.sizes();
        // Bound retained allocation. Normal chunk planes contain 256 samples.
        if sx * sz > 1 << 20 {
            return Err(DecodeError::Field("material preliminary plane size"));
        }
        let [x, _, z] = expected.minimum();
        let plane = DensityVolume::new([sx as i32, 1, sz as i32], [x, 0, z], [1; 3])
            .map_err(|_| DecodeError::Field("material preliminary plane"))?;
        Ok(Self {
            surface,
            expected,
            densities: Vec::new(),
            retained_samples: 0,
            plane,
            sampler,
            context,
            buffer: None,
        })
    }
}
impl<S: MaterialSurfaceQueries> MaterialSurfaceQueries for MaterialDensitySources<S> {
    fn bind_density(
        &mut self,
        sampler: Arc<dyn DensitySampler>,
        eager: bool,
    ) -> Result<usize, DecodeError> {
        if self.densities.len() >= 32768
            || (eager
                && (self.expected.size() > 1 << 20
                    || self.retained_samples + self.expected.size() > 1 << 24))
        {
            return Err(DecodeError::Field("material density storage bound"));
        }
        let buffer = if eager {
            let mut buffer = vec![0.0; self.expected.size()];
            sampler.sample_volume(&mut self.context, &mut buffer, self.expected)?;
            self.retained_samples += buffer.len();
            Some(buffer)
        } else {
            None
        };
        let index = self.densities.len();
        self.densities.push(BoundDensity { sampler, buffer });
        Ok(index)
    }
    fn density(&mut self, index: usize, position: BlockPos) -> Result<f32, MaterialRuleError> {
        let source = self
            .densities
            .get(index)
            .ok_or(MaterialRuleError::MissingDensityProvider)?;
        let position = [position.x, position.y, position.z];
        if let Some(buffer) = &source.buffer {
            if let Some(index) = self.expected.index_of_block(position) {
                return Ok(buffer[index]);
            }
        }
        Ok(source.sampler.sample_value(&mut self.context, position)?)
    }
    fn preliminary_surface(&mut self, position: BlockPos) -> Result<f32, MaterialRuleError> {
        let position = [position.x, 0, position.z];
        if let Some(index) = self.plane.index_of_block(position) {
            if self.buffer.is_none() {
                // Retain the allocation and partial writes if sampling fails.
                let buffer = self.buffer.insert(vec![0.0; self.plane.size()]);
                self.sampler
                    .sample_volume(&mut self.context, buffer, self.plane)?;
            }
            Ok(self.buffer.as_ref().unwrap()[index])
        } else {
            Ok(self.sampler.sample_value(&mut self.context, position)?)
        }
    }
    fn possible_biomes(&self) -> Option<BTreeSet<String>> {
        self.surface.possible_biomes()
    }
    fn biome(&mut self, position: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
        self.surface.biome(position)
    }
    fn melts_frozen_ocean(
        &mut self,
        biome: &str,
        position: BlockPos,
        sea_level: i32,
    ) -> Result<bool, MaterialRuleError> {
        self.surface.melts_frozen_ocean(biome, position, sea_level)
    }
    fn cold_enough_to_snow(
        &mut self,
        biome: &str,
        position: BlockPos,
        sea_level: i32,
    ) -> Result<bool, MaterialRuleError> {
        self.surface.cold_enough_to_snow(biome, position, sea_level)
    }
    fn bind_noise(&mut self, name: &str) -> Result<Arc<dyn Noise>, DecodeError> {
        self.surface.bind_noise(name)
    }
    fn surface_depth(&mut self, x: i32, z: i32) -> i32 {
        self.surface.surface_depth(x, z)
    }
    fn surface_secondary(&mut self, x: i32, z: i32) -> f64 {
        self.surface.surface_secondary(x, z)
    }
    fn band(&mut self, position: BlockPos) -> Result<BlockStateId, MaterialRuleError> {
        self.surface.band(position)
    }
}
