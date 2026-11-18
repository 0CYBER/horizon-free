//! `/time`.
use super::host::{CommandHost, R, int};
use horizon_client::{
    command::{Arg, Context},
    text::Text,
};
use horizon_core::java_number::float_to_string;

/// What `/time` needs from the game: the clocks it names and the world's time.
pub trait TimeHost: CommandHost {
    /// `Level.getGameTime`.
    fn game_time(&self) -> i64;
    /// The default clock of the level's dimension type, if it has one.
    fn default_clock(&self) -> Option<String>;
    /// The id of the level in place.
    fn dimension_id(&self) -> &'static str;
    /// `ServerClockManager.getTotalTicks` of `clock`.
    fn clock_time(&self, clock: &str) -> i64;
    /// Sets the total ticks of `clock`.
    fn set_clock_time(&mut self, clock: &str, total: i64);
    /// Whether `clock` is paused. Like every read of a clock's state, it makes the clock known.
    fn clock_paused(&mut self, clock: &str) -> bool;
    fn set_clock_paused(&mut self, clock: &str, paused: bool);
    /// The rate of `clock`.
    fn clock_rate(&mut self, clock: &str) -> f32;
    fn set_clock_rate(&mut self, clock: &str, rate: f32);
    /// Drops the partial tick of `clock`.
    fn reset_clock_partial(&mut self, clock: &str);
    /// A mod's `time_skip` listeners, for a command jumping the selected clock from `from` to
    /// `to` (the Overworld's or any other the command names): true when one cancels the jump.
    fn time_skip_canceled(&mut self, from: i64, to: i64) -> bool;
    /// The Overworld clock's time marker `marker` seen from `total`: the time of its next
    /// occurrence, and whether it is occurring now. None for a marker that is unknown or never comes.
    fn time_marker(&self, marker: &str, total: i64) -> Option<(i64, bool)>;
    /// The period of the Overworld clock's timeline `timeline` (None inside when it repeats
    /// never), if the timeline exists.
    fn timeline_period(&self, timeline: &str) -> Option<Option<i64>>;
}

/// `/time`: `id` is the command node, such as `time.set` or `time.query.gametime`.
pub fn time(host: &mut dyn TimeHost, id: &str, ctx: &Context) -> R<i32> {
    if id == "time.query.gametime" {
        let t = host.game_time();
        host.feedback_unbroadcast(Text::tr("commands.time.query.gametime", &[&t]));
        return Ok((t % i64::from(i32::MAX)) as i32);
    }
    // `time of <clock>` names the clock; otherwise the dimension's default clock.
    let clock = match ctx.get("clock") {
        Some(Arg::Resource(c)) => c.clone(),
        _ => match host.default_clock() {
            Some(clock) => clock,
            None => {
                return Err(host.error("commands.time.no_default_clock", &[&host.dimension_id()]));
            }
        },
    };
    let total = host.clock_time(&clock);
    match id {
        "time.set" => {
            let t = i64::from(int(ctx, "time").unwrap_or(0));
            if total == t {
                return Err(host.error("commands.time.set.already_at_time", &[&clock, &t]));
            }
            if !host.time_skip_canceled(total, t) {
                host.set_clock_time(&clock, t);
                host.reset_clock_partial(&clock);
            }
            host.feedback(Text::tr("commands.time.set.absolute", &[&clock, &t]));
            Ok(t as i32)
        }
        "time.set.marker" => {
            let Some(Arg::Resource(marker)) = ctx.get("timemarker").cloned() else {
                return Ok(0);
            };
            let Some((target, occurring)) = (clock == "minecraft:overworld")
                .then(|| host.time_marker(&marker, total))
                .flatten()
            else {
                return Err(host.error("commands.time.no_time_marker_found", &[&marker, &clock]));
            };
            if occurring {
                return Err(host.error(
                    "commands.time.set.already_at_time_marker",
                    &[&clock, &marker],
                ));
            }
            if !host.time_skip_canceled(total, target) {
                host.set_clock_time(&clock, target);
                host.reset_clock_partial(&clock);
            }
            host.feedback(Text::tr(
                "commands.time.set.time_marker",
                &[&clock, &marker],
            ));
            Ok((host.clock_time(&clock) % i64::from(i32::MAX)) as i32)
        }
        "time.add" => {
            let t = i64::from(int(ctx, "time").unwrap_or(0));
            let target = (total + t).max(0);
            if !host.time_skip_canceled(total, target) {
                host.set_clock_time(&clock, target);
            }
            let now = host.clock_time(&clock);
            host.feedback(Text::tr("commands.time.set.absolute", &[&clock, &now]));
            Ok((now % i64::from(i32::MAX)) as i32)
        }
        "time.pause" | "time.resume" => {
            let pause = id == "time.pause";
            if host.clock_paused(&clock) == pause {
                return Err(host.error(
                    if pause {
                        "commands.time.pause.already_paused"
                    } else {
                        "commands.time.pause.already_running"
                    },
                    &[&clock],
                ));
            }
            host.set_clock_paused(&clock, pause);
            host.feedback(Text::tr(
                if pause {
                    "commands.time.pause"
                } else {
                    "commands.time.resume"
                },
                &[&clock],
            ));
            Ok(1)
        }
        "time.rate" => {
            let rate = match ctx.get("rate") {
                Some(Arg::Float(r)) => *r,
                _ => 1.0,
            };
            if host.clock_rate(&clock) == rate {
                return Err(host.error(
                    "commands.time.rate.already_same",
                    &[&clock, &float_to_string(rate)],
                ));
            }
            host.set_clock_rate(&clock, rate);
            host.feedback(Text::tr(
                "commands.time.rate",
                &[&clock, &float_to_string(rate)],
            ));
            Ok(1)
        }
        "time.query.time" => {
            host.feedback_unbroadcast(Text::tr("commands.time.query.absolute", &[&clock, &total]));
            Ok((total % i64::from(i32::MAX)) as i32)
        }
        "time.query.timeline" | "time.query.repetition" => {
            let Some(Arg::Resource(timeline)) = ctx.get("timeline").cloned() else {
                return Ok(0);
            };
            let Some(period) = (clock == "minecraft:overworld")
                .then(|| host.timeline_period(&timeline))
                .flatten()
            else {
                return Err(host.error(
                    "commands.time.wrong_timeline_for_clock",
                    &[&timeline, &clock],
                ));
            };
            if id == "time.query.timeline" {
                let ticks = period.map_or(total, |p| total.rem_euclid(p));
                host.feedback_unbroadcast(Text::tr(
                    "commands.time.query.timeline",
                    &[&timeline, &ticks],
                ));
                Ok(ticks as i32)
            } else {
                let reps = period.map_or(0, |p| total / p);
                host.feedback_unbroadcast(Text::tr(
                    "commands.time.query.timeline.repetitions",
                    &[&timeline, &reps],
                ));
                Ok(reps as i32)
            }
        }
        _ => Ok(0),
    }
}
