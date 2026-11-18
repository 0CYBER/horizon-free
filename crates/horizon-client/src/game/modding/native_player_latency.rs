//! `player_latency`: the ping the Tab list shows for a player.
use super::Game;
use mod_api::PlayerRef;
use std::sync::atomic::Ordering;

impl Game<'_> {
    /// The player's keep-alive latency in milliseconds
    /// (`ServerCommonPacketListenerImpl.latency`), 0 for an acting player with no known
    /// connection; None for a stale or unavailable player.
    pub(super) fn player_latency(&self, player: PlayerRef) -> Option<i32> {
        if let Some(guest) = self.online_player_record(player) {
            return Some(guest.latency.load(Ordering::Relaxed));
        }
        (self.native_acting_player() == Some(player)).then_some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use std::sync::atomic::Ordering;

    /// The latency is the one the connection's keep-alives measure, per player.
    #[test]
    #[ignore = "daily"]
    fn player_latency_reads_the_connections_keep_alive_average() {
        with_guests(|game| {
            let a = game.player(12).unwrap();
            let b = game.player(13).unwrap();
            let hub = &game.lan.as_ref().unwrap().hub;
            hub.guest(12).unwrap().latency.store(87, Ordering::Relaxed);
            assert_eq!(game.player_latency(a), Some(87));
            assert_eq!(game.player_latency(b), Some(0));
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert_eq!(game.player_latency(gone), None);
        });
    }
}
