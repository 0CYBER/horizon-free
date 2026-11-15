//! The `chunk_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0066

pub trait Hooks {
    /// Chunk `chunk` of `dimension` left the world (NeoForge's
    /// `ChunkEvent.Unload`), with a level that closes for being empty too. Don't
    /// read its blocks. Its block entities are still there, but changes made in
    /// this hook aren't saved: as in vanilla, the chunk is saved before it unloads.
    /// Not told when the world closes, which `world_lifecycle` tells.
    /// Server-side.
    #[hook(side = "server", contract = "listen", neoforge = "ChunkEvent.Unload")]
    fn chunk_unload();
}
