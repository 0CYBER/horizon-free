use crate::game::Game;
use horizon_core::random::LevelRandom;
use horizon_gameplay::commands::weather::WeatherHost;

impl WeatherHost for Game<'_> {
    fn command_random(&mut self) -> &mut LevelRandom {
        &mut self.world.random
    }
    fn set_weather_parameters(&mut self, clear: i32, time: i32, raining: bool, thundering: bool) {
        Game::set_weather_parameters(self, clear, time, raining, thundering);
    }
}
