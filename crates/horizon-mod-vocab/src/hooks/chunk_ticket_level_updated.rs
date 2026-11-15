//! The `chunk_ticket_level_updated` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `chunk`'s ticket level went from `old` to `new` (NeoForge's
    /// `ChunkTicketLevelUpdatedEvent`, from `ChunkMap.updateChunkScheduling`):
    /// a player, a forced chunk, a portal or the dragon fight's arena began or
    /// ceased to keep it alive or ticking. It is the level of the chunk's
    /// loading tickets (`DistanceManager`'s chunk tracker), not the
    /// simulation distance's. A level is a `ChunkTicketLevel`: the number
    /// `ChunkLevel` gives, 31 and lower entity ticking, 32 block ticking and 33
    /// loaded without ticking, or `Unloaded`; none is clamped to the ticking
    /// range. Never told with `old == new`.
    ///
    /// Told in order, once for each time a chunk's level changed: a chunk that
    /// changes twice in a tick is told twice, the second `old` the first `new`.
    /// Within one update of the level's tickets the chunks come by position (x,
    /// then z), which is not vanilla's order. A new level's tickets tell every
    /// chunk they keep loaded as coming from `Unloaded`. Told once a tick, in
    /// the server's tick of the level, after the tickets were updated for
    /// every player there; a level with no player in it is not ticked and
    /// tells nothing until it is. NeoForge's `chunkHolder` is not given:
    /// Horizon keeps no chunk holders. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ChunkTicketLevelUpdatedEvent"
    )]
    fn chunk_ticket_level_updated();
}
