use super::{Dim, Game};
use mod_api::{AttachmentHolder, EntityRef};

impl Game<'_> {
    /// Publish a placed player's saved map, or bind before its first store.
    /// Scope changes always replace the registration, even for an emptied map.
    pub(in crate::game) fn bind_player_attachments(&mut self, conn: u32, first_store: bool) {
        if !self.native_authoritative() {
            return;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return;
        };
        let Some(guest) = self.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .connection_index(conn)
                .map(|i| &host.hub.guests[i])
        }) else {
            return;
        };
        if !guest.placed || !guest.participates_online() {
            return;
        }
        let dim = Dim::from_id(&guest.dimension);
        let uuid = guest.uuid;
        let holder = AttachmentHolder::Player(mod_api::__private::game::player_ref(
            EntityRef::new(epoch, guest.entity_id as u32),
            uuid,
        ));
        let acting = self.lan_acting_uuid == Some(uuid);
        if !acting && self.lan_guest_swaps.contains(&conn) {
            return;
        }
        let map = if acting {
            &mut self.player_state.attachments
        } else {
            let Some(map) = self
                .lan
                .as_mut()
                .and_then(|host| host.hub.guests.attachments_mut(conn))
            else {
                return;
            };
            map
        };
        if first_store && map.has_index_binding() {
            return;
        }
        if first_store || map.has_index_entries() || map.has_index_binding() {
            map.bind_index(self.attachment_indexes.entry(dim).or_default(), holder);
        }
    }
}
