//! FEATURES status over a 3×3 neighborhood of generated chunks.
use super::{
    anchor::WorldGenerationContext,
    biome_source::ResolvedBiomeSource,
    generator::{DecorateStats, GeneratorData, GeneratorError, ProtoChunk, WorkerState},
};
use std::sync::Arc;

pub(super) struct Decoration;
impl Decoration {
    pub(super) fn new(
        _data: &Arc<GeneratorData>,
        _source: &ResolvedBiomeSource,
    ) -> Result<Option<Self>, GeneratorError> {
        Ok(None)
    }
    pub(super) fn decorate(
        &self,
        _data: &Arc<GeneratorData>,
        _seed: i64,
        _generation: WorldGenerationContext,
        _worker: &mut WorkerState,
        _region: &mut [ProtoChunk; 9],
    ) -> Result<DecorateStats, GeneratorError> {
        Ok(DecorateStats::default())
    }
}
