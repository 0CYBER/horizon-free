use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `player` started watching `chunk` (NeoForge's `ChunkWatchEvent.Watch`):
    /// the chunk came into their view and goes to their client, once until
    /// `native_chunk_unwatch`. Vanilla tells it as the chunk is queued
    /// (`ChunkMap.markChunkPendingToSend`); Horizon picks a chunk only as it
    /// sends it, so this comes just before `native_chunk_sent` for the same
    /// chunk. Told after the server's network step for the level, with the
    /// state after that step: the order per player and chunk is vanilla's.
    /// Not told for the far ring's columns past the view distance. Server-side.
    pub(in crate::game) fn native_chunk_watch(&mut self, player: PlayerRef, chunk: ChunkRef) {
        self.fire_chunk_watch(player, chunk);
    }
    /// `chunk`'s packet went to `player` (NeoForge's `ChunkWatchEvent.Sent`,
    /// at the end of `PlayerChunkSender.sendChunk`), after
    /// `native_chunk_watch`, so a mod can send its own data for the chunk
    /// now. Told after the server's network step, as `native_chunk_watch`.
    /// Not told for the far ring's columns. Server-side.
    pub(in crate::game) fn native_chunk_sent(&mut self, player: PlayerRef, chunk: ChunkRef) {
        self.fire_chunk_sent(player, chunk);
    }
    /// `player` stopped watching `chunk` (NeoForge's `ChunkWatchEvent.UnWatch`,
    /// `ChunkMap.dropChunk`): it left their view, or they leave the level
    /// (through a portal or a teleport, or respawning) or the server. Once
    /// for each chunk `native_chunk_watch` told of. After a disconnect the
    /// player handle no longer reads. Told after the server's network step,
    /// as `native_chunk_watch`. Not told when the server stops. Server-side.
    pub(in crate::game) fn native_chunk_unwatch(&mut self, player: PlayerRef, chunk: ChunkRef) {
        self.fire_chunk_unwatch(player, chunk);
    }
}
