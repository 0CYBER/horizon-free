//! Bind actual network members to the existing residency journal.
use super::*;

impl Game<'_> {
    pub(super) fn bind_guest_residency(&self, hub: &mut Hub) {
        hub.payload_player_epoch = self.native_lifecycle.epoch();
        hub.guests
            .bind_residency(&self.resident_entities.table.borrow().producer(), |id| {
                crate::game::Dim::from_id(id).key()
            });
    }
}

impl Game<'_> {
    /// The start of a phase vanilla runs before ChunkMap.tick and Horizon
    /// among the entities' ticks (ServerChunkCache.tickChunks' spawning and
    /// thunder, the raids): the counter to give `lan_synced_since`.
    pub(in crate::game) fn lan_synced_from(&mut self) -> Option<i32> {
        self.lan_ids_so_far()
    }
}
