//! The `client_chunk_load` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` came into this
    /// client's level from the server (NeoForge's `ChunkEvent.Load` on the
    /// client, whose `isNewChunk` is always false there): once per chunk
    /// packet the client adopts, after the column and its block entities are
    /// in the level, so a listener reads its blocks through `ClientHost`. A
    /// column the server sends again while it is resident is told again.
    /// Only on a client of a server, a LAN guest included; an integrated
    /// world's own chunks have the server's `chunk_load`. Client-side.
    #[notes = r##"## Client chunk load

`client_chunk_load(dimension, chunk)` runs on a client of a server, a LAN
guest included, once for each `LevelChunkWithLight` packet the client adopts
(vanilla's `ClientChunkCache.replaceWithPacketData`). It runs after the column
and its block entities are in the client level, so a listener can read the
column's blocks through `ClientHost` (`host.level(dimension)`, then
`level.block_state(host, pos)`). A column the server sends again while it is
resident is told again, without an unload between. A biome packet
(`ChunksBiomes`) tells nothing. `dimension` is the client level's dimension, as
the server named it. An integrated world's own chunks have the server's
`chunk_load` instead, and that hook is not called on the client."##]
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ChunkEvent.Load",
        cancellable = false
    )]
    fn client_chunk_load();
}
