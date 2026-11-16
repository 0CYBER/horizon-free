//! The `chunk_load` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0065

pub trait Hooks {
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` came into the world
    /// (NeoForge's `ChunkEvent.Load`): `new` when it was just generated, false
    /// when read from the save. A flat world's chunk that leaves before it's
    /// first lit isn't saved, so it's generated, and new, again; one generated
    /// as the players left is new when it next comes, unless the server
    /// restarts first, or the level closes for being empty, which drops its
    /// memory of what never saved. Its blocks and block entities are there,
    /// but its saved entities join afterwards and its light may not be ready:
    /// change the world from a later tick. The chunks around the spawn come
    /// before `server_started`. Not told of a chunk a client receives.
    /// The chunks leaving in the same tick were already copied for their last
    /// save: an edit made here to one of them (a neighbour's block entity, say)
    /// is not in that copy, and is dropped with the chunk.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ChunkEvent.Load",
        paper = "ChunkLoadEvent"
    )]
    fn chunk_load();
}
