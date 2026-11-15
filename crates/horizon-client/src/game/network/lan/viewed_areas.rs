//! The players' viewed areas (arismoko/horizon#3716, `lan::viewed_levels`) in the level in place: the squares
//! `Streaming.extra` keeps loaded for them (`lan_update_streaming`) and their own tickets
//! (`Sources::viewed`). Neither goes through the forced chunks or the mods' chunk tickets. With no
//! area on the server both are empty, without allocating.
use super::Game;
use horizon_client::ticket_levels::ViewedTicket;

impl Game<'_> {
    /// The squares the areas in the level in place keep loaded and generated.
    pub(super) fn viewed_extra(&self) -> Vec<((i32, i32), i32)> {
        self.lan
            .as_ref()
            .map_or_else(Vec::new, |h| h.hub.viewed_extra(self.dim.id()))
    }

    /// The areas' tickets in the level in place.
    pub(in crate::game) fn viewed_tickets(&self) -> Vec<ViewedTicket> {
        self.lan
            .as_ref()
            .map_or_else(Vec::new, |h| h.hub.viewed_tickets(self.dim.id()))
    }
}
