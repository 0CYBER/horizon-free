//! A mod's view centre for one player (horizon-rust/horizon#19219): the chunk their chunk tracking
//! and loading follow in place of their own, in their own level, without moving them. Vanilla's
//! `ServerPlayer.setCamera` moves the player to the camera so that `ChunkMap.move` follows it; a
//! camera mod (a drone, a cinematic camera) wants the chunks without the move.

use super::Hub;
use crate::lan::viewed_levels::areas::MAX_CENTER;

/// Why [`Hub::set_view_center`] changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewCenterRefusal {
    /// No online player with that id.
    NotOnline,
    /// A centre more than [`MAX_CENTER`] chunks from (0, 0) on an axis.
    OutOfRange,
}

impl Hub {
    /// Centres the chunk tracking of the player `uuid` on `center`, a chunk of their own level,
    /// or on their own chunk again with `None`. The chunks around the centre are loaded and sent
    /// as around the player, to their view distance, and their client is told the new chunk cache
    /// centre; what leaves the view is forgotten as when they move. The player stays where they
    /// are. The centre ends at a respawn and a level change.
    pub fn set_view_center(
        &mut self,
        uuid: u128,
        center: Option<(i32, i32)>,
    ) -> Result<(), ViewCenterRefusal> {
        let Some(i) = self
            .guests
            .iter()
            .position(|g| g.uuid == uuid && g.participates_online())
        else {
            return Err(ViewCenterRefusal::NotOnline);
        };
        let far = |c: i32| !(-MAX_CENTER..=MAX_CENTER).contains(&c);
        if center.is_some_and(|(x, z)| far(x) || far(z)) {
            return Err(ViewCenterRefusal::OutOfRange);
        }
        // The next tick's `update_view` sees the centre move: it sends the cache centre, forgets
        // what left the view and queues what entered it, closest first.
        self.guests.update(i, |g| g.view_center = center);
        Ok(())
    }

    /// The view centre a mod set for the player `uuid`; none for their own chunk or no such
    /// player.
    pub fn view_center(&self, uuid: u128) -> Option<(i32, i32)> {
        self.guests
            .iter()
            .find(|g| g.uuid == uuid)
            .and_then(|g| g.view_center)
    }
}
