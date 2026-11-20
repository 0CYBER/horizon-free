//! `entity_set_visible_to`: an entity hidden from one player's client alone.
use super::Game;
use mod_api::{EntityRef, EntityVisibilityRefusal as Refusal, PlayerRef};

impl Game<'_> {
    /// The viewer's UUID and the entity's, once both name something in the server's list. A viewer's
    /// own player is no entity to hide from them: the client is never sent its own player.
    fn visibility_pair(
        &mut self,
        entity: EntityRef,
        viewer: PlayerRef,
    ) -> Result<(u128, u128), Refusal> {
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        let epoch = self.native_lifecycle.epoch();
        if epoch != Some(viewer.entity().epoch()) {
            return Err(Refusal::Stale);
        }
        let hub = &self.lan.as_ref().ok_or(Refusal::Stale)?.hub;
        match hub
            .guests
            .iter()
            .find(|g| g.uuid == viewer.uuid() && g.placed)
        {
            None => return Err(Refusal::Stale),
            Some(g) if g.is_fake() => return Err(Refusal::NoConnection),
            Some(_) => {}
        }
        if epoch != Some(entity.epoch()) {
            return Err(Refusal::StaleEntity);
        }
        let uuid = self.entity_uuid(entity).ok_or(Refusal::StaleEntity)?;
        if uuid == viewer.uuid() {
            return Err(Refusal::StaleEntity);
        }
        Ok((viewer.uuid(), uuid))
    }

    /// Hides the entity from the viewer's client, or shows it again; whether that changed it.
    pub(super) fn entity_set_visible_to(
        &mut self,
        entity: EntityRef,
        viewer: PlayerRef,
        visible: bool,
    ) -> Result<bool, Refusal> {
        let (viewer, entity) = self.visibility_pair(entity, viewer)?;
        let host = self.lan.as_mut().ok_or(Refusal::Stale)?;
        host.hub
            .set_entity_hidden_from(viewer, entity, !visible)
            .ok_or(Refusal::NoConnection)
    }

    /// Whether `entity_set_visible_to` left the entity visible to the viewer.
    pub(super) fn entity_visible_to(
        &mut self,
        entity: EntityRef,
        viewer: PlayerRef,
    ) -> Result<bool, Refusal> {
        let (viewer, entity) = self.visibility_pair(entity, viewer)?;
        let host = self.lan.as_ref().ok_or(Refusal::Stale)?;
        host.hub
            .entity_hidden_from(viewer, entity)
            .map(|hidden| !hidden)
            .ok_or(Refusal::NoConnection)
    }
}
