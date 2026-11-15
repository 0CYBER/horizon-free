use super::*;

impl Game<'_> {
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` came into this
    /// client's level from the server (NeoForge's `ChunkEvent.Load` on the
    /// client), after its column and block entities are in the level.
    /// Client-side.
    pub(in crate::game) fn native_client_chunk_load(&mut self, dimension: &str, chunk: [i32; 2]) {
        self.fire_client_chunk_load(dimension, chunk);
    }
}
