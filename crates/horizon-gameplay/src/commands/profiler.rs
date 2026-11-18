//! `/debug start|stop` (the tick profiler) and `/jfr start|stop`.
use super::host::{CommandHost, R};
use horizon_client::{
    command::{CommandError, Context},
    text::{Style, Text},
};

/// What the profiler commands need from the game: the recordings it keeps and its tick count.
pub trait ProfilerHost: CommandHost {
    /// `MinecraftServer.isTimeProfilerRunning`: the tick profiler runs or is about to start.
    fn profiler_running(&self) -> bool;
    /// `startTimeProfiler`: the profiler starts with the next tick.
    fn start_profiler_delayed(&mut self);
    /// `stopTimeProfiler`: takes the profiler's run, as the seconds it ran and the ticks that
    /// passed. None when it has not started yet (the delayed start stays pending).
    fn take_profiler_run(&mut self) -> Option<(f64, f64)>;
    /// `/debug function`, which stays with the game.
    fn debug_function(&mut self, ctx: &Context) -> R<i32>;
    /// Whether a `/jfr start` recording is running.
    fn jfr_running(&self) -> bool;
    /// Starts the `/jfr` recording, named by `stamp`, from the tick count now.
    fn start_jfr(&mut self, stamp: String);
    /// Takes the `/jfr` recording: the seconds it ran, the ticks that passed and its stamp.
    fn take_jfr(&mut self) -> Option<(f64, u64, String)>;
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
fn tr_error(key: &str, args: &[Text]) -> CommandError {
    CommandError::new(Text::translate(key, args))
}
/// `String.format(Locale.ROOT, "%.2f", v)`.
fn two_places(v: f64) -> String {
    horizon_core::java_number::format_fixed(v, 2)
}
/// `Util.getFilenameFormattedDateTime` (`yyyy-MM-dd_HH.mm.ss`, local time).
pub fn filename_date_time() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}.{mi:02}.{s:02}")
}
/// UTC civil date and time of a Unix timestamp.
fn civil(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `/debug start|stop|function`.
pub fn debug(host: &mut dyn ProfilerHost, ctx: &Context) -> R<i32> {
    if ctx.literals.iter().any(|l| l == "function") {
        return host.debug_function(ctx);
    }
    // MinecraftServer.isTimeProfilerRunning / startTimeProfiler /
    // stopTimeProfiler: a stop before the delayed start took effect
    // reports empty results and leaves the start pending.
    let running = host.profiler_running();
    if ctx.literals.iter().any(|l| l == "start") {
        if running {
            return Err(host.error("commands.debug.alreadyRunning", &[]));
        }
        host.start_profiler_delayed();
        host.feedback(Text::translate("commands.debug.started", &[]));
        return Ok(0);
    }
    if !running {
        return Err(host.error("commands.debug.notRunning", &[]));
    }
    let (seconds, ticks) = host.take_profiler_run().unwrap_or((0.0, 0.0));
    let tps = ticks / seconds;
    host.feedback(Text::translate(
        "commands.debug.stopped",
        &[
            Text::literal(two_places(seconds)),
            num(ticks as i64),
            Text::literal(two_places(tps)),
        ],
    ));
    Ok(tps as i32)
}

/// `/jfr start|stop` (JfrCommand). Horizon has no JVM flight recorder:
/// the recording is its own tick timing, dumped as JSON.
pub fn jfr(host: &mut dyn ProfilerHost, ctx: &Context) -> R<i32> {
    if ctx.literals.iter().any(|l| l == "start") {
        if host.jfr_running() {
            return Err(host.error("commands.jfr.start.failed", &[]));
        }
        let stamp = filename_date_time();
        host.start_jfr(stamp);
        host.feedback(Text::translate("commands.jfr.started", &[]));
        return Ok(1);
    }
    let Some((seconds, ticks, stamp)) = host.take_jfr() else {
        return Err(tr_error(
            "commands.jfr.dump.failed",
            &[Text::literal("Not currently profiling")],
        ));
    };
    let relative = format!("debug/client-{stamp}.json");
    let path = std::env::current_dir().unwrap_or_default().join(&relative);
    let report = serde_json::json!({
        "environment": "client",
        "seconds": seconds,
        "ticks": ticks,
    });
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, report.to_string()));
    if let Err(e) = written {
        return Err(tr_error(
            "commands.jfr.dump.failed",
            &[Text::literal(e.to_string())],
        ));
    }
    let file = Text::literal(relative).underlined().with_style(&Style {
        hover: Some(std::sync::Arc::new(Text::translate("chat.copy.click", &[]))),
        ..Style::default()
    });
    host.feedback(Text::translate("commands.jfr.stopped", &[file]));
    Ok(1)
}
