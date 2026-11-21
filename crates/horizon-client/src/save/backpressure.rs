//! Backpressure for asynchronous world saves.
use super::*;

const MAX_ASYNC_CHUNK_WRITES: usize = 512;

impl SaveDir {
    pub(super) fn encoding_for_save_later(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<Arc<Encoding>> {
        biomes.ensure_saveable()?;
        if !chunks.is_empty() && self.active_chunk_writes() >= MAX_ASYNC_CHUNK_WRITES {
            self.flush()?;
        }
        Ok(Arc::new(Encoding {
            registry,
            biomes: biomes.clone(),
            tables: self.tables.get().cloned(),
            shape: self.shape,
            section_palettes: Some(Arc::clone(&self.section_palettes)),
        }))
    }
}

/// Each record encoded and compressed (`None` for a deletion), in the order
/// given. Chunk compression and NBT encoding are offloaded across a persistent
/// background worker pool, keeping region file writing and header flushes
/// ordered, atomic, and thread-safe.
pub(super) fn compress_records(
    records: &[((i32, i32), &Record)],
    timestamp: u32,
) -> Vec<std::io::Result<Option<ChunkRecord>>> {
    super::compression_pool::compress_records(records, timestamp)
}
