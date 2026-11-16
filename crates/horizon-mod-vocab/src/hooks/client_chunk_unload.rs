//! The `client_chunk_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` is leaving this
    /// client's level (NeoForge's `ChunkEvent.Unload` on the client): once
    /// per resident column, with its blocks still there, just before they are
    /// freed. Told on every route a column leaves by: the server's forget
    /// packet, the view centre moving away, the view radius shrinking, and
    /// the client level going (a dimension change, a respawn into another
    /// level, leaving the server). That is wider than NeoForge, which tells
    /// only the forget packet, so a listener never keeps state for a column
    /// that is gone. Only on a client of a server, a LAN guest included; an
    /// integrated world's own chunks have the server's `chunk_unload`.
    /// Client-side.
    #[notes = r##"## Client chunk unload

`client_chunk_unload(dimension, chunk)` runs on a client of a server once
for each column that leaves the client level, while its blocks and block
entities are still readable, just before they are freed. Every route a column
leaves by tells it: the server's `ForgetLevelChunk` packet, the view centre
moving away, the view radius shrinking, and the client level going (a dimension
change, a respawn into another level, leaving the server). A column that is
already gone is not told again. NeoForge tells only the forget packet
(`ClientChunkCache.drop`); Horizon tells every route, so a listener never keeps
state for a column the client no longer holds. `dimension` is the level the
column belonged to: on a dimension change, the old one."##]
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ChunkEvent.Unload",
        cancellable = false
    )]
    fn client_chunk_unload();
}
