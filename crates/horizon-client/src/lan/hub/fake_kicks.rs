//! Connectionless players finish a kick at the same poll boundary as network leaves.

use super::{HostEvent, Hub};

impl Hub {
    pub(super) fn finish_fake_kicks(&mut self, events: &mut Vec<HostEvent>) {
        // Visit only queued kicks, without scanning the player list on ordinary ticks.
        // Keep each Kicked before its Left, as for a connection closed in this poll.
        for event in 0..events.len() {
            let HostEvent::Kicked { conn, .. } = &events[event] else {
                continue;
            };
            let Some(index) = self.guests.connection_index(*conn) else {
                continue;
            };
            if !self.guests[index].is_fake() || !self.guests[index].disconnect_pending {
                continue;
            }
            let guest = self.remove_guest(index, "");
            events.push(HostEvent::Left(Box::new(guest)));
        }
    }
}
