//! What a server mod reads of the tick times and sets of the view and simulation distance.
use super::Game;

impl Game<'_> {
    /// `MinecraftServer.getTickTimesNanos`.
    pub(super) fn tick_times(&self) -> Vec<u64> {
        self.clock.tick_times()
    }
    /// `PlayerList.getViewDistance`.
    pub(super) fn server_view_distance(&self) -> Option<i32> {
        Some(self.lan.as_ref()?.hub.config.view_distance)
    }
    /// `PlayerList.getSimulationDistance`.
    pub(super) fn server_simulation_distance(&self) -> Option<i32> {
        Some(self.lan.as_ref()?.hub.config.simulation_distance)
    }
    /// `PlayerList.setViewDistance` on a dedicated server; the tickets read the settings.
    pub(super) fn server_set_view_distance(&mut self, distance: i32) -> bool {
        if !(2..=32).contains(&distance) || self.options.dedicated.is_none() || self.lan.is_none() {
            return false;
        }
        self.set_view_distance(distance);
        self.settings.render_distance = distance;
        true
    }
    /// `PlayerList.setSimulationDistance` on a dedicated server.
    pub(super) fn server_set_simulation_distance(&mut self, distance: i32) -> bool {
        if !(2..=32).contains(&distance) || self.options.dedicated.is_none() || self.lan.is_none() {
            return false;
        }
        self.set_simulation_distance(distance);
        self.settings.simulation_distance = distance;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// The tick times are the clock's 100; the distances are the hub's, set live on a dedicated
    /// server only, within 2 to 32.
    #[test]
    #[ignore = "daily"]
    fn a_mod_reads_and_sets_the_servers_distances() {
        with_guests(|game| {
            game.clock = crate::clock::FixedStep::default();
            for took in [40_000_000, 51_000_000, 62_000_000] {
                game.clock.record(std::time::Duration::from_nanos(took));
            }
            let host: &dyn mod_api::ServerHost = game;
            let times = host.tick_times();
            assert_eq!(times.len(), 100);
            assert_eq!(times[..4], [40_000_000, 51_000_000, 62_000_000, 0]);
            assert_eq!(game.server_view_distance(), Some(2));
            assert_eq!(game.server_simulation_distance(), Some(2));
            assert!(!game.server_set_view_distance(6), "an integrated server");
            game.options.dedicated = Some(Vec::new());
            assert!(game.server_set_view_distance(6));
            assert!(game.server_set_simulation_distance(5));
            assert!(!game.server_set_view_distance(1));
            assert!(!game.server_set_simulation_distance(33));
            assert_eq!(game.server_view_distance(), Some(6));
            assert_eq!(game.server_simulation_distance(), Some(5));
            assert_eq!(game.settings.simulation_distance, 5);
        });
    }
}
