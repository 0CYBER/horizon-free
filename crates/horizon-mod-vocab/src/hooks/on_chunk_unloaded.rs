//! The `on_chunk_unloaded` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The chunk holding this mod block entity is unloading
    /// (`IBlockEntityExtension.onChunkUnloaded`): told once for each of its
    /// mod block entities, on each side that holds a copy of it, before the
    /// copy is dropped. `pos` is where it stands, `block_entity` its handle (the
    /// one `host.block_entity` gave while it was loaded) and `entity` its data.
    /// Release what your mod keeps for it here. On a server its chunk may have
    /// left the level already, so don't count on the handle's reads. As
    /// with `chunk_unload`, changes made in it aren't saved: the chunk
    /// was saved before it unloads. Not told when the world closes, or on a
    /// client when its level goes (a respawn, a dimension change, leaving).
    #[notes = r##"### When a block entity's chunk unloads

`on_chunk_unloaded` is NeoForge's `IBlockEntityExtension.onChunkUnloaded`: a `BlockEntityPart` attached to a mod
block entity type is told once for each block entity of that type in a chunk that unloads, so a machine with a
running job, an open handle or a place in a network it keeps can let go of it. `event.pos` is where it stands,
`event.block_entity` its handle, equal to the one `host.block_entity` gave while it was loaded (so a map keyed by
it finds the entry), and `event.entity` its type id and data as they are at the call: on a server, what the chunk
was just saved with; on a client, its copy as the server last sent it.

It fires on each side that holds a copy of the block entity. On a server it fires for each chunk that leaves a
level, a level that closes for being empty included, after the chunk was saved and before the block entity is
dropped. A chunk the server streams out has already left the level by then, so the handle's reads and writes
find nothing there (`None`, `Stale`): read what you need from the event. Changes made here, through the handle or
otherwise, aren't saved, as for `chunk_unload`. On a client (the window's own in single player included)
it fires when the server forgets a chunk or the view moves away from it, while the chunk is still there.

It fires once per block entity per unload and never for a vanilla block entity. It does not fire when the world
closes (that moment has `world_lifecycle`), nor on a client when its whole level goes (a respawn, a
dimension change, leaving the server). The chunk-wide notice stays `chunk_unload`, a server hook: on a server
it is told after the chunk's block entities, and a client is told of them alone. A chunk unload with no part
subscribed costs one check."##]
    #[hook(
        side = "both",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "IBlockEntityExtension.onChunkUnloaded"
    )]
    fn on_chunk_unloaded();
}
