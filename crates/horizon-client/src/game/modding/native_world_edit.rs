//! Bulk block edits for server mods: a helper on the level being ticked, as `native_set_block` is, that makes many
//! runs of blocks up columns at once, straight into the chunks.
use super::Game;

/// The most chunks one `level.relight_chunks` call lights (each brings its ring of eight).
const RELIGHT_MAX: usize = 256;

impl Game<'_> {
    /// For a server mod: the runs made at once, straight into the chunks of the authoritative level `dimension`
    /// (the one being ticked, as for `native_set_block`), with what `fill ... strict` does for block entities (a
    /// block replaced by another loses its block entity, and a new block gets the one `native_set_block` would
    /// give it, a mod block's included); no neighbour is told unless `neighbour_updates`, and the chunks changed
    /// go to the players whole.
    /// Runs in chunks not loaded, and the parts of runs outside the column, are left out. The blocks changed, or
    /// why the edit was left out, with nothing changed: another dimension, a client of a server, a palette entry
    /// or filter that doesn't read or carries NBT (refused), a run naming a palette entry or filter past the end,
    /// or more than 65536 filters.
    pub(super) fn edit_blocks(
        &mut self,
        dimension: &str,
        edit: &mod_api::BlockEdit,
    ) -> Result<u64, String> {
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        let started = std::time::Instant::now();
        let changed = self.apply_block_edit(edit)?;
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self.options.spikes.is_some_and(|threshold| ms > threshold) {
            log::info!(
                "Horizon: slow mod block edit {ms:.1} ms ({} runs, {changed} blocks)",
                edit.runs.len()
            );
        }
        Ok(changed)
    }
    /// The level in place, or why a chunk write to `level` is refused: a client, a stale
    /// handle, a level that is not the one being ticked.
    fn chunk_write_gate(&self, level: mod_api::LevelRef) -> Result<(), mod_api::ChunkRefusal> {
        use mod_api::{ChunkRefusal, LevelReadError};
        match self.native_level_world_result(level) {
            Err(LevelReadError::Unavailable) => Err(ChunkRefusal::Client),
            Err(_) => Err(ChunkRefusal::Stale),
            Ok(_) if level.dimension() != self.dim.key() => Err(ChunkRefusal::NotInPlace),
            Ok(_) => Ok(()),
        }
    }
    /// `chunks` of the level in place, and the loaded ring around them, lit again now
    /// (`Streaming::relight_now`): how many were lit.
    pub(super) fn level_relight_chunks(
        &mut self,
        level: mod_api::LevelRef,
        chunks: &[[i32; 2]],
    ) -> Result<u32, mod_api::ChunkRefusal> {
        self.chunk_write_gate(level)?;
        if chunks.len() > RELIGHT_MAX {
            return Err(mod_api::ChunkRefusal::TooManyChunks {
                asked: chunks.len().min(u32::MAX as usize) as u32,
                max: RELIGHT_MAX as u32,
            });
        }
        let mut columns: Vec<(i32, i32)> = chunks
            .iter()
            .flat_map(|&[x, z]| {
                (-1..=1).flat_map(move |dz| {
                    (-1..=1).map(move |dx| (x.wrapping_add(dx), z.wrapping_add(dz)))
                })
            })
            .filter(|key| self.world.chunks.contains_key(key))
            .collect();
        columns.sort_unstable();
        columns.dedup();
        Ok(self.streaming.relight_now(&mut self.world, &columns) as u32)
    }
    /// The chunk queued to go whole to its watchers at the end of the tick, as an
    /// `edit_blocks` chunk does (`World::bulk_log`).
    pub(super) fn chunk_resend(
        &mut self,
        chunk: mod_api::ChunkRef,
    ) -> Result<(), mod_api::ChunkRefusal> {
        self.chunk_write_gate(chunk.level())?;
        let [x, z] = chunk.pos();
        if !self.world.chunks.contains_key(&(x, z)) {
            return Err(mod_api::ChunkRefusal::NotLoaded);
        }
        if self.world.block_log.is_some() {
            self.world.bulk_log.insert((x, z));
        }
        Ok(())
    }
    /// Whether `dimension` is the authoritative level being ticked: where a server mod's world helpers act.
    pub(super) fn native_level_open(&self, dimension: &str) -> bool {
        (self.serving.is_some() || self.options.dedicated.is_some()) && dimension == self.dim.id()
    }
}
