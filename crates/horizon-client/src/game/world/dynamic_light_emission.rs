//! `light_emission`, the light a `dynamic_light` block emits by where it is
//! (`IBlockExtension.getLightEmission`): asked of its parts on the thread that owns the
//! level as a chunk holding one comes in, as an edit or `level_check_light` reaches one, and
//! on a client as the server's light update reaches its section, the answers kept in the
//! chunk (`Chunk::dynamic_emission`) for the light engine, which never asks on its own.
use super::*;
use horizon_client::fast_hash::FxHashMap;

impl Game<'_> {
    /// The chunks the workers brought into this level in this update, asked before any
    /// light job reads them (`Chunk::dynamic_pending`).
    pub(super) fn ask_loaded_light_emission(&mut self) {
        if !self.pack.context.light.any_dynamic() {
            return;
        }
        for i in 0..self.world.loaded_chunks.len() {
            let (x, z, _) = self.world.loaded_chunks[i];
            self.ask_chunk_light_emission((x, z));
        }
    }
    /// Every `dynamic_light` cell of the resident chunk `key` asked, its answers replacing
    /// the chunk's, and the chunk let through to the light jobs. A server's column is asked
    /// as it is taken in, before any light job can read it.
    pub(super) fn ask_chunk_light_emission(&mut self, key: (i32, i32)) {
        let Some(chunk) = self.world.chunks.get(&key) else {
            return;
        };
        let pending = chunk.dynamic_pending;
        let cells = self.pack.context.light.dynamic_cells(chunk, |_| true);
        if cells.is_empty() && !pending {
            return;
        }
        let dimension = self.dim.level();
        let mut answers = FxHashMap::default();
        for (cell, [x, y, z], state) in cells {
            if let Some(emission) = self.ask_light_emission(dimension, x, y, z, state) {
                answers.insert(cell, emission);
            }
        }
        if let Some(chunk) = self.world.chunks.get_mut(&key)
            && (pending || chunk.dynamic_emission != answers)
        {
            // Saved light was not computed from these answers, and admission queues no light
            // job for it: relight the column when they differ.
            let relight = chunk.light_loaded && chunk.dynamic_emission != answers;
            let chunk = Arc::make_mut(chunk);
            chunk.dynamic_emission = answers;
            chunk.dynamic_pending = false;
            if relight {
                self.world.dirty_light.insert(key);
            }
        }
    }
    /// The cells this update's edits and light checks left in a `dynamic_light` state,
    /// asked before the light engine relights them (`Streaming::relight_edits`).
    /// `Chunk::set` already dropped the answers of the states they replaced.
    pub(super) fn ask_edited_light_emission(&mut self) {
        // The checks queued so far are asked now: a later one at the same cell queues again.
        self.world.light_checks.clear();
        if self.world.light_edits.is_empty() || !self.pack.context.light.any_dynamic() {
            return;
        }
        let dimension = self.dim.level();
        for i in 0..self.world.light_edits.len() {
            let ((x, y, z), _, _) = self.world.light_edits[i];
            let state = self.world.block(x, y, z).0;
            if !self.pack.context.light.is_dynamic(state) {
                continue;
            }
            let emission = self.ask_light_emission(dimension, x, y, z, state);
            let key = (x >> 4, z >> 4);
            if let Some(cell) = self
                .world
                .chunks
                .get(&key)
                .and_then(|c| c.dynamic_cell(x, y, z))
            {
                self.store_light_emission(key, cell, emission);
            }
        }
    }
    /// A server's light update changed the light of `sections` of the column `key`: their
    /// `dynamic_light` cells asked again, each once, so this copy's own relights take what the
    /// server's took (the server asked before it relit them). A Horizon addition: the contract
    /// reads emission again on no light packet, but a part's answer may read data the client
    /// receives later.
    pub(super) fn ask_updated_light_emission(&mut self, key: (i32, i32), sections: &[usize]) {
        if sections.is_empty() || !self.pack.context.light.any_dynamic() {
            return;
        }
        let Some(chunk) = self.world.chunks.get(&key) else {
            return;
        };
        let cells = self
            .pack
            .context
            .light
            .dynamic_cells(chunk, |s| sections.contains(&s));
        let dimension = self.dim.level();
        for (cell, [x, y, z], state) in cells {
            let emission = self.ask_light_emission(dimension, x, y, z, state);
            self.store_light_emission(key, cell, emission);
        }
    }
    /// One cell's answer stored in its chunk (`None` drops the one it had); the chunk is
    /// copied only when the answer changes.
    fn store_light_emission(&mut self, key: (i32, i32), cell: u32, emission: Option<u8>) {
        let Some(chunk) = self.world.chunks.get_mut(&key) else {
            return;
        };
        if chunk.dynamic_emission.get(&cell).copied() == emission {
            return;
        }
        let answers = &mut Arc::make_mut(chunk).dynamic_emission;
        match emission {
            Some(emission) => answers.insert(cell, emission),
            None => answers.remove(&cell),
        };
    }
    /// One cell's answer, clamped to a light level as `LightTables` clamps a state's.
    fn ask_light_emission(
        &self,
        dimension: LevelDimension,
        x: i32,
        y: i32,
        z: i32,
        state: u32,
    ) -> Option<u8> {
        let mut emission = None;
        self.native_light_emission(
            dimension,
            BlockPos::new(x, y, z),
            BlockStateId(state),
            &mut emission,
        );
        emission.map(|level| level.min(15))
    }
}
