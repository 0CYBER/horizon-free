use super::*;

impl Game<'_> {
    /// The time of day and the rain and thunder levels this client draws with, `partial` ticks past
    /// the last tick: the level's (or PvP & HUD's) as they come in. A client mod may set them to
    /// show this player another time or weather; only the picture follows.
    pub(in crate::game) fn native_sky_environment(
        &self,
        partial: f32,
        day_time: &mut i64,
        rain: &mut f32,
        thunder: &mut f32,
    ) {
        self.fire_sky_environment(partial, day_time, rain, thunder);
    }
}

#[cfg(test)]
mod sky_environment_tests {
    use mod_api::{ClientHost, Hooks, ModEntry, SkyEnvironmentEvent};
    use mod_platform::testing::with_mods;

    /// Noon in rain for this window, whatever the world's time and weather; a thunder level that
    /// isn't finite.
    struct NoonRain;
    #[mod_api::__subscriptions]
    impl Hooks for NoonRain {
        fn sky_environment(&self, _: &dyn ClientHost, event: &mut SkyEnvironmentEvent) {
            event.day_time = event.day_time.div_euclid(24000) * 24000 + 6000;
            event.rain = 1.0;
            event.thunder = f32::NAN;
        }
    }

    /// A client mod's time and weather are what the frame draws; the world's clock and weather
    /// stay, a level that isn't finite keeps the world's, and the game that ticks the world
    /// draws its own.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_draws_its_own_time_and_weather() {
        use crate::game::native_client_session::tests::logged_client;
        use crate::game::native_entity_refs::tests::{with_client_game, with_game};
        let mods: mod_api::Mods = vec![ModEntry::new("noon-rain", Box::new(NoonRain))].into();
        with_mods(mods, || {
            with_client_game(|game| {
                let _server = logged_client(game);
                game.world.day_time = 42_000;
                let world_weather = game.weather_levels(0.5);
                assert_eq!(game.view_day_time(), 30_000, "noon of the same day");
                assert_eq!(game.view_weather(0.5), (1.0, world_weather.1));
                assert_eq!(game.world.day_time, 42_000, "the world's clock stays");
                assert_eq!(game.weather_levels(0.5), world_weather, "and its weather");
            });
            with_game(|game| {
                game.world.day_time = 42_000;
                assert_eq!(game.view_day_time(), 42_000, "only a client is asked");
            });
        });
    }
}
