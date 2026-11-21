//! `entity_range_percent`: the entity broadcast range the server hosts with.
use super::Game;

impl Game<'_> {
    /// The hosted server's entity broadcast range in percent (`hub.entity_range_percent`),
    /// 100 when no server hosts a world.
    pub(super) fn entity_range_percent(&self) -> i32 {
        self.lan
            .as_ref()
            .map_or(100, |host| host.hub.entity_range_percent)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// The range reads the hub's value: 100 by default, and what the server set after that.
    #[test]
    #[ignore = "daily"]
    fn entity_range_percent_reads_the_hubs_range() {
        with_guests(|game| {
            assert_eq!(game.entity_range_percent(), 100);
            game.lan.as_mut().unwrap().hub.entity_range_percent = 50;
            assert_eq!(game.entity_range_percent(), 50);
        });
    }
}
