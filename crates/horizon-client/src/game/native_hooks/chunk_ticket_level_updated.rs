use super::*;

/// The hook point. It does nothing in Horizon.
impl Game<'_> {
    /// `chunk`'s ticket level went from `old` to `new` (NeoForge's
    /// `ChunkTicketLevelUpdatedEvent`): a ticket began or ceased to keep it
    /// alive or ticking. Told in the order the level's tickets changed it,
    /// once a tick after they were updated, never with `old == new`.
    /// Server-side.
    pub(in crate::game) fn native_chunk_ticket_level_updated(
        &mut self,
        chunk: ChunkRef,
        old: mod_api::ChunkTicketLevel,
        new: mod_api::ChunkTicketLevel,
    ) {
        self.fire_chunk_ticket_level_updated(chunk, old, new);
    }
}
