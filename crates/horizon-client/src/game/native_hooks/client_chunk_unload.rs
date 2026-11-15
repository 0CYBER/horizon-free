use super::*;

impl Game<'_> {
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` is leaving this
    /// client's level (NeoForge's `ChunkEvent.Unload` on the client), with its
    /// blocks still there, whichever route frees it. Client-side.
    pub(in crate::game) fn native_client_chunk_unload(&mut self, dimension: &str, chunk: [i32; 2]) {
        self.fire_client_chunk_unload(dimension, chunk);
    }
}
