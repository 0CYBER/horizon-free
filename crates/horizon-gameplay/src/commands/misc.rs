//! `/stopwatch` and `/tick`.
use super::host::{CommandHost, R};
use horizon_client::{
    clock::{FixedStep, SprintReport},
    command::{Arg, Context},
    stopwatches::Stopwatches,
    text::Text,
};
use horizon_core::java_number::format_fixed;

/// What `/stopwatch` and `/tick` need from the game.
pub trait MiscHost: CommandHost {
    /// The world's stopwatches (`CommandStorage` of `minecraft:stopwatches`).
    fn stopwatches(&mut self) -> &mut Stopwatches;
    /// The game clock `/tick` reads and changes.
    fn tick_clock(&mut self) -> &mut FixedStep;
    /// `TickCommand.nanosToMilisString`: nanoseconds as a one-decimal millisecond string.
    fn millis(&self, nanos: u64) -> String;
    /// `/tick rate`'s `setTickRate` and `onTickRateChanged` (the hook, and the autosave wait shortened).
    fn apply_tick_rate(&mut self, rate: f32);
    /// The report `/tick sprint` prints when a sprint ends.
    fn sprint_report(&mut self, report: &SprintReport);
    /// `MinecraftServer.onTickRateChanged`.
    fn on_tick_rate_changed(&mut self);
}

/// `/stopwatch` (StopwatchCommand).
pub fn stopwatch(host: &mut dyn MiscHost, id: &str, ctx: &Context) -> R<i32> {
    let Some(Arg::Resource(name)) = ctx.get("id").cloned() else {
        return Ok(0);
    };
    let arg = || Text::literal(name.clone());
    let missing = |host: &dyn MiscHost| host.error("commands.stopwatch.does_not_exist", &[&name]);
    match id {
        "stopwatch.create" => {
            if !host.stopwatches().create(&name) {
                return Err(host.error("commands.stopwatch.already_exists", &[&name]));
            }
            host.feedback(Text::translate(
                "commands.stopwatch.create.success",
                &[arg()],
            ));
            Ok(1)
        }
        "stopwatch.query" => {
            let scale = match ctx.get("scale") {
                Some(Arg::Double(v)) => *v,
                _ => 1.0,
            };
            let Some(t) = host.stopwatches().elapsed_seconds(&name) else {
                return Err(missing(&*host));
            };
            host.feedback(Text::translate(
                "commands.stopwatch.query",
                &[arg(), Text::literal(format!("{t:?}"))],
            ));
            Ok((t * scale) as i32)
        }
        "stopwatch.restart" => {
            if !host.stopwatches().restart(&name) {
                return Err(missing(&*host));
            }
            host.feedback(Text::translate(
                "commands.stopwatch.restart.success",
                &[arg()],
            ));
            Ok(1)
        }
        _ => {
            if !host.stopwatches().remove(&name) {
                return Err(missing(&*host));
            }
            host.feedback(Text::translate(
                "commands.stopwatch.remove.success",
                &[arg()],
            ));
            Ok(1)
        }
    }
}

/// `/tick` (TickCommand) over the game clock.
pub fn tick(host: &mut dyn MiscHost, id: &str, ctx: &Context) -> R<i32> {
    let t = |key: &str, args: &[Text]| Text::translate(key, args);
    let lit = |s: String| Text::literal(s);
    let time = match ctx.get("time") {
        Some(Arg::Int(v)) => *v,
        _ => 1,
    };
    match id {
        "tick.rate" => {
            let rate = match ctx.get("rate") {
                Some(Arg::Float(v)) => *v,
                _ => 20.0,
            };
            // MinecraftServer.setTickRate: the autosave wait already
            // running is shortened when the new rate asks for less.
            host.apply_tick_rate(rate);
            host.feedback(t(
                "commands.tick.rate.success",
                &[lit(format_fixed(f64::from(rate), 1))],
            ));
            Ok(rate as i32)
        }
        "tick.query" => {
            // TickCommand.tickQuery: every line passes `broadcast` false.
            let rate = host.tick_clock().rate();
            let average = host.tick_clock().average_nanos();
            let busy = host.millis(average);
            let rate_text = format_fixed(f64::from(rate), 1);
            if host.tick_clock().sprinting() {
                host.feedback_unbroadcast(t("commands.tick.status.sprinting", &[]));
                host.feedback_unbroadcast(t(
                    "commands.tick.query.rate.sprinting",
                    &[lit(rate_text), lit(busy)],
                ));
            } else {
                let target = host.tick_clock().tick_duration().as_nanos() as u64;
                let status = if host.tick_clock().frozen() {
                    "commands.tick.status.frozen"
                } else if target < host.tick_clock().average_nanos() {
                    "commands.tick.status.lagging"
                } else {
                    "commands.tick.status.running"
                };
                host.feedback_unbroadcast(t(status, &[]));
                let target_text = host.millis(target);
                host.feedback_unbroadcast(t(
                    "commands.tick.query.rate.running",
                    &[lit(rate_text), lit(busy), lit(target_text)],
                ));
            }
            let samples = host.tick_clock().sorted_samples();
            let at = |q: f32| {
                samples
                    .get(((samples.len() as f32 * q) as usize).min(samples.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or(0)
            };
            let (p50, p95, p99) = (
                host.millis(at(0.5)),
                host.millis(at(0.95)),
                host.millis(at(0.99)),
            );
            host.feedback_unbroadcast(t(
                "commands.tick.query.percentiles",
                &[lit(p50), lit(p95), lit(p99), lit(samples.len().to_string())],
            ));
            Ok(rate as i32)
        }
        "tick.sprint" => {
            if host.tick_clock().start_sprint(time as u32) {
                host.feedback(t("commands.tick.sprint.stop.success", &[]));
            }
            host.feedback(t("commands.tick.status.sprinting", &[]));
            Ok(1)
        }
        "tick.sprint.stop" => {
            let Some(report) = host.tick_clock().stop_sprint() else {
                return Err(host.error("commands.tick.sprint.stop.fail", &[]));
            };
            host.sprint_report(&report);
            host.on_tick_rate_changed();
            host.feedback(t("commands.tick.sprint.stop.success", &[]));
            Ok(1)
        }
        "tick.freeze" | "tick.unfreeze" => {
            let freeze = id == "tick.freeze";
            if freeze {
                if let Some(report) = host.tick_clock().stop_sprint() {
                    host.sprint_report(&report);
                    host.on_tick_rate_changed();
                }
                host.tick_clock().stop_step();
            }
            host.tick_clock().set_frozen(freeze);
            host.feedback(t(
                if freeze {
                    "commands.tick.status.frozen"
                } else {
                    "commands.tick.status.running"
                },
                &[],
            ));
            Ok(i32::from(freeze))
        }
        "tick.step" => {
            if !host.tick_clock().start_step(time as u32) {
                return Err(host.error("commands.tick.step.fail", &[]));
            }
            host.feedback(t("commands.tick.step.success", &[lit(time.to_string())]));
            Ok(1)
        }
        _ => {
            if !host.tick_clock().stop_step() {
                return Err(host.error("commands.tick.step.stop.fail", &[]));
            }
            host.feedback(t("commands.tick.step.stop.success", &[]));
            Ok(1)
        }
    }
}
