//! What a server mod's diagnostics read (a profiler's mob costs, PacketEvents' counters): the
//! time each mob type takes and each player's packets. Nothing is kept unless a mod asks.
use super::{Game, native_server::PlayerRead};
use horizon_client::lan::packet_counts;
use mod_api::{MobTickCost, PacketCount, PlayerRef};

impl Game<'_> {
    /// The mod's own mob tick sampling (`tick_profile::set_mod_sampling`), apart from the
    /// report's. False on a client.
    pub(super) fn mob_cost_sampling(&mut self, on: bool) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        super::tick_profile::set_mod_sampling(on);
        true
    }
    /// What the mod's sampling summed since the last call, costliest first. Empty on a client.
    pub(super) fn take_mob_costs(&mut self) -> Vec<MobTickCost> {
        if !self.is_logical_server() {
            return Vec::new();
        }
        super::tick_profile::take_mod()
            .into_iter()
            .map(|c| MobTickCost::new(c.dimension.to_owned(), c.kind, c.nanos, c.mob_ticks))
            .collect()
    }
    /// The server's packet counting switch (`packet_counts::set_counting`). False on a client.
    pub(super) fn count_packets(&mut self, on: bool) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        packet_counts::set_counting(on);
        true
    }
    /// The server stopped: packet counting and the mod's mob sampling go off, so they cannot carry
    /// into a later server of this process.
    pub(in crate::game) fn stop_mod_diagnostics(&mut self) {
        packet_counts::set_counting(false);
        super::tick_profile::set_mod_sampling(false);
    }
    /// A connected player's packet counts since the last take. None on a client, for a stale
    /// or unavailable player and for one with no connection.
    pub(super) fn player_take_packet_counts(
        &mut self,
        player: PlayerRef,
    ) -> Option<Vec<PacketCount>> {
        if !self.is_logical_server() {
            return None;
        }
        let conn = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.lan_acting?,
            PlayerRead::Guest(g) => g.conn,
        };
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        if guest.is_fake() {
            return None;
        }
        Some(
            guest
                .sender
                .packet_counts
                .take()
                .into_iter()
                .map(|(outbound, id, c)| PacketCount::new(outbound, id, c.packets, c.bytes))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::lan::packet_counts;

    /// A guest's sent packets are counted while counting is on, taken once; the mod's mob
    /// sampling switches on and off with nothing sampled.
    #[test]
    #[ignore = "daily"]
    fn a_players_packets_are_counted_while_asked() {
        with_guests(|game| {
            let b = game.player(13).unwrap();
            assert!(game.count_packets(true));
            let sender = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(13)
                .unwrap()
                .sender
                .clone();
            sender.send(vec![0x2b, 1, 2]);
            sender.send(vec![0x2b]);
            let counts = game.player_take_packet_counts(b).unwrap();
            assert!(game.count_packets(false));
            let ours = counts
                .iter()
                .find(|c| c.outbound && c.id == 0x2b)
                .expect("counted");
            assert_eq!((ours.packets, ours.bytes), (2, 4));
            assert!(game.player_take_packet_counts(b).unwrap().is_empty());
            sender.send(vec![0x2b]);
            assert!(game.player_take_packet_counts(b).unwrap().is_empty());
            assert!(!packet_counts::counting());
            assert!(game.mob_cost_sampling(true));
            assert!(game.mob_cost_sampling(false));
            assert!(game.take_mob_costs().is_empty());
        });
    }

    /// A server that stops with counting and sampling on leaves both off for the next one.
    #[test]
    #[ignore = "daily"]
    fn stopping_the_server_switches_the_diagnostics_off() {
        with_guests(|game| {
            assert!(game.count_packets(true));
            assert!(game.mob_cost_sampling(true));
            assert!(packet_counts::counting());
            game.stop_serving();
            assert!(!packet_counts::counting());
        });
    }
}
