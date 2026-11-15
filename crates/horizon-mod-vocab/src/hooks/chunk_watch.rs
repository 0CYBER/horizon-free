//! The `chunk_watch` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` started watching `chunk` (NeoForge's `ChunkWatchEvent.Watch`):
    /// the chunk came into their view and goes to their client, once until
    /// `chunk_unwatch`. Vanilla tells it as the chunk is queued
    /// (`ChunkMap.markChunkPendingToSend`); Horizon picks a chunk only as it
    /// sends it, so this comes just before `chunk_sent` for the same chunk.
    /// Told after the server's network step for the level, and a handler sees
    /// the state after that step; the order per player and chunk is vanilla's.
    /// Not told for the far ring's columns past the view distance (a Horizon
    /// server option). Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ChunkWatchEvent.Watch"
    )]
    fn chunk_watch();
    /// `chunk`'s packet went to `player` (NeoForge's `ChunkWatchEvent.Sent`,
    /// at the end of `PlayerChunkSender.sendChunk`), after `chunk_watch`, so
    /// a mod can send its own data for the chunk now. Told after the server's
    /// network step for the level, with the state after that step, as
    /// `chunk_watch`. Not told for the far ring's columns. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ChunkWatchEvent.Sent"
    )]
    fn chunk_sent();
    /// `player` stopped watching `chunk` (NeoForge's `ChunkWatchEvent.UnWatch`,
    /// `ChunkMap.dropChunk`): it left their view, or they leave the level
    /// (through a portal or a teleport, or respawning) or the server. Once for
    /// each chunk `chunk_watch` told of. After a disconnect the player handle
    /// no longer reads. Told after the server's network step for the level,
    /// with the state after that step, as `chunk_watch`. Not told when the
    /// server stops. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ChunkWatchEvent.UnWatch"
    )]
    fn chunk_unwatch();
}
