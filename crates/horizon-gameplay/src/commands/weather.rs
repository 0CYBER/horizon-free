//! `/weather`.
use super::host::{CommandHost, R, int};
use horizon_client::{command::Context, text::Text};
use horizon_core::random::{LevelRandom, RandomSource};

/// What `/weather` needs from the game.
pub trait WeatherHost: CommandHost {
    /// The level's random, which `/weather` draws a default duration from.
    fn command_random(&mut self) -> &mut LevelRandom;
    /// `MinecraftServer.setWeatherParameters(clear, time, raining, thundering)` in the level in place.
    fn set_weather_parameters(&mut self, clear: i32, time: i32, raining: bool, thundering: bool);
}

/// `/weather clear|rain|thunder [duration]`.
pub fn weather(host: &mut dyn WeatherHost, ctx: &Context) -> R<i32> {
    let duration = int(ctx, "duration");
    let sample = |min: i32, max: i32, r: &mut horizon_core::random::LegacyRandom| {
        min + r.next_int_bound(max - min + 1).unwrap_or(0)
    };
    let (key, clear, time, raining, thundering) = if ctx.has("clear") {
        (
            "commands.weather.set.clear",
            duration.unwrap_or_else(|| sample(12_000, 180_000, host.command_random())),
            0,
            false,
            false,
        )
    } else if ctx.has("rain") {
        (
            "commands.weather.set.rain",
            0,
            duration.unwrap_or_else(|| sample(12_000, 24_000, host.command_random())),
            true,
            false,
        )
    } else {
        (
            "commands.weather.set.thunder",
            0,
            duration.unwrap_or_else(|| sample(3_600, 15_600, host.command_random())),
            true,
            true,
        )
    };
    host.set_weather_parameters(clear, time, raining, thundering);
    host.feedback(Text::translate(key, &[]));
    Ok(duration.unwrap_or(-1))
}
