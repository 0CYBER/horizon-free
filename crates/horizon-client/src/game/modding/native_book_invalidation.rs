//! Reloads and authoritative knowledge changes revoke published display identities.
use super::*;
use horizon_client::recipe_book::placement_sync::Frame;
impl Game<'_> {
    fn invalidate_mod_book_placement_connection(&mut self, conn: u32) {
        let Some(server) = self.native_net.placement.servers.get_mut(&conn) else {
            return;
        };
        server.clear();
        if let Some(epoch) = self
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .map(|authority| authority.epoch)
        {
            self.send_guest_mod_book_placement(conn, Frame::Invalidate { epoch });
        }
    }
    pub(super) fn invalidate_mod_book_placement_catalogues(&mut self) {
        let connections: Vec<_> = self.native_net.placement.servers.keys().copied().collect();
        for conn in connections {
            self.invalidate_mod_book_placement_connection(conn);
        }
        self.clear_mod_book_placement_screen();
        self.ensure_mod_book_catalog();
    }
    pub(super) fn invalidate_mod_book_placement_knowledge(&mut self) {
        let Some(uuid) = self.lan_acting_uuid else {
            return;
        };
        let connection = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|guest| guest.uuid == uuid))
            .map(|guest| guest.conn);
        if let Some(conn) = connection {
            self.invalidate_mod_book_placement_connection(conn);
        }
    }
}
