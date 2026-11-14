//! A travel into a viewed level (#3716, 9e): `ServerPlayer.teleport(TeleportTransition)`'s
//! Respawn (keep 3) into a level the player's client views swaps the two on both sides, as
//! Immersive Portals moves the client's world reference with no loading screen. The chunks of the
//! level entered that its client was sent stay sent, now as its own; the level left becomes a
//! viewed level that keeps every chunk sent, lingering until an area covers it again. Nothing is
//! sent again but the entities of both levels, which the client keeps apart from its worlds.
//!
//! It is negotiated: only a client that answered revision 2 of `horizon:levels`
//! (`viewed_levels::REVISION_2`) swaps. There is no marker: such a client swaps on that Respawn
//! exactly when it holds the level entered as a viewed world, which is when this server has its
//! view open. A travel of a revision-1 client or into a level the client does not view, and a
//! death's or an End exit's Respawn (`PlayerList.respawn`), drops every view instead, and the
//! client every viewed world.
use super::super::{
    chunk_watch::ChunkWatchKind,
    hub::{AttachmentSnapshot, Hub, tracked},
};
use super::view::View;
use horizon_mod_runtime::attachments::Holder;

impl Hub {
    /// `change_level`'s part once guest `i` was moved from level `left` into the level in place:
    /// its views swapped with its own chunks when its client swaps worlds (revision 2) and views
    /// the level in place, else its views dropped. Either way it leaves `left`'s queues and
    /// entities.
    pub(in crate::lan) fn leave_level(&mut self, i: usize, left: &str) {
        // Its client names the level left by its index at Login: one opened since has none.
        let left_level = self.guests[i].viewed.login_index(left);
        let entered = self.config.dimension.clone();
        let watching = self.chunk_watches.recording();
        let kept = self.guests.update(i, |g| {
            let own = std::mem::take(&mut g.sent_chunks);
            g.far_sent_chunks.clear();
            g.chunk_queue.clear();
            g.far_queue.clear();
            // The client drops both levels' entities: the server sends them again.
            g.tracker.clear();
            let switches = g.sender.viewed_levels.switches() && left_level.is_some();
            let (Some(level), Some(view)) = (
                left_level,
                switches.then(|| g.viewed.views.remove(&entered)).flatten(),
            ) else {
                g.viewed.forget_views();
                return Vec::new();
            };
            // The level left keeps what its client holds; the next pass starts its linger.
            let mut old = View::new(level);
            old.sent = own;
            g.viewed.views.insert(left.to_owned(), old);
            // What the view sent is the player's own now: `update_view` forgets what is past the
            // player's view and queues the rest.
            g.sent_chunks = view.sent;
            let mut kept: Vec<_> = g.sent_chunks.iter().copied().collect();
            kept.sort_unstable();
            if watching {
                let ((cx, cz), r) = (g.chunk(), g.view_distance);
                for &key in kept.iter().filter(|&&(x, z)| tracked(cx, cz, r, x, z)) {
                    if g.watched.insert(key) {
                        self.chunk_watches.push(g, ChunkWatchKind::Watch, key);
                    }
                }
            }
            kept
        });
        // A viewed level's chunks went without their synced attachments.
        if self.sync_attachments {
            let player = self.guests[i].uuid;
            self.attachment_snapshots
                .extend(kept.into_iter().map(|(x, z)| AttachmentSnapshot {
                    player,
                    holder: Holder::Chunk([x, z]),
                }));
        }
    }
}
