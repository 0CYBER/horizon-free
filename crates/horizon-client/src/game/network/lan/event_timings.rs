use super::LanHost;
use std::time::Instant;

pub(super) use horizon_gameplay::lan::event_timings::*;

/// Notes how long the event that started at `started` took (`spans::note`), with what it
/// was about when `Game::event_detail` said.
pub(super) fn note_event(
    host: &LanHost,
    started: Option<(Instant, EventKind, Option<UseDetail>)>,
    timings: &mut EventTimings<(Option<u32>, Option<String>), UseDetail>,
) {
    let Some((started, kind, detail)) = started else {
        return;
    };
    let ms = started.elapsed().as_secs_f32() * 1000.0;
    horizon_client::spans::note(ms, || describe_event(host, &kind, detail.as_ref()));
    let (name, conn, player) = kind;
    timings.add(name, (conn, player), detail, ms);
}

/// The player an event came from: its name, else its connection.
fn who(host: &LanHost, conn: Option<u32>, name: Option<String>) -> Option<String> {
    name.or_else(|| conn.and_then(|c| host.hub.guest(c)).map(|g| g.name.clone()))
        .or_else(|| conn.map(|c| format!("connection {c}")))
}

/// A burst's note: `213 x UseItemOn under 10 ms each (…); most from Steve: 140 of them, 710.0 ms`.
pub(super) fn describe_burst(
    host: &LanHost,
    burst: &Burst<(Option<u32>, Option<String>), UseDetail>,
) -> String {
    let mut line = format!(
        "{} x {} under {} ms each",
        burst.count,
        burst.kind,
        horizon_client::spans::NOTE_MS
    );
    if let Some(d) = &burst.detail {
        line.push_str(&format!(" ({d})"));
    }
    if let Some(((conn, name), count, ms)) = &burst.top
        && let Some(who) = who(host, *conn, name.clone())
    {
        line.push_str(&format!("; most from {who}: {count} of them, {ms:.1} ms"));
    }
    line
}

pub(super) fn describe_event(
    host: &LanHost,
    (kind, conn, name): &EventKind,
    detail: Option<&UseDetail>,
) -> String {
    let line = match who(host, *conn, name.clone()) {
        Some(who) => format!("{kind} of {who}"),
        None => (*kind).to_owned(),
    };
    match detail {
        Some(d) => format!("{line} ({d})"),
        None => line,
    }
}
