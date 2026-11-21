//! `viewer_entity_*`: client-only entities one viewer sees, not in the level.
use super::Game;
use mod_api::PlayerRef;

impl Game<'_> {
    /// Spawns a client-only entity of `kind` at `pos` for `viewer` alone; its id, None when refused
    /// (see the host declaration). Takes the next world entity id, so an id is used up even when refused.
    pub(super) fn viewer_entity_spawn(
        &mut self,
        viewer: PlayerRef,
        kind: &str,
        pos: [f64; 3],
    ) -> Option<u32> {
        if !pos.iter().all(|c| c.is_finite()) {
            return None;
        }
        if self.lan.is_none() || self.entity_ids_exhausted() {
            return None;
        }
        // The one world entity counter (`Entity.nextEntityId`), so the id never names a tracked entity.
        let id = self.take_entity_id();
        let host = self.lan.as_mut()?;
        host.hub
            .spawn_viewer_entity(viewer.uuid(), id, kind, pos)
            .then_some(id)
    }

    /// Moves a client-only entity of `viewer` to `pos`.
    pub(super) fn viewer_entity_move(&mut self, viewer: PlayerRef, id: u32, pos: [f64; 3]) -> bool {
        if !pos.iter().all(|c| c.is_finite()) {
            return false;
        }
        let Some(host) = &mut self.lan else {
            return false;
        };
        host.hub.move_viewer_entity(viewer.uuid(), id, pos)
    }

    /// Removes a client-only entity of `viewer`.
    pub(super) fn viewer_entity_remove(&mut self, viewer: PlayerRef, id: u32) -> bool {
        let Some(host) = &mut self.lan else {
            return false;
        };
        host.hub.remove_viewer_entity(viewer.uuid(), id)
    }
}
