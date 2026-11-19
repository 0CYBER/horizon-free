//! A Horizon client and a Horizon server of different builds that can't agree on Horizon's own
//! protocol (`horizon:payloads`): the refusal names both builds, and which one to update, instead
//! of the negotiation's error alone.
//!
//! Each end sends its build on [`CHANNEL`] before its first `horizon:payloads` message, so the
//! other end has it before anything there can fail. Vanilla clients discard the payload, as they
//! discard the negotiation's Query; builds from before this never send it, which is how an end
//! knows the other is older.
use crate::{lang::Language, text::Text};
use std::cmp::Ordering;

pub const CHANNEL: &str = "horizon:build";
/// The refusal's key: the auto-reconnect never retries it.
pub const KEY: &str = "horizon.multiplayer.build_mismatch";
/// The longest build name read from the other end.
const MAX_BUILD: usize = 128;

/// The build this binary is, as the crash report and the title screen name it.
pub fn this_build() -> String {
    crate::crash::version()
}

pub fn encode(build: &str) -> Vec<u8> {
    let mut w = horizon_protocol::buf::Writer::new();
    w.string(build);
    w.data
}

pub fn decode(data: &[u8]) -> Option<String> {
    horizon_protocol::buf::Reader::new(data)
        .string(MAX_BUILD)
        .ok()
}

/// The refusal for a failed negotiation between `server` and `client`; `None` for a build that
/// never announced itself, so one older than this code. `detail` is the negotiation's error, kept
/// on its own line for reports. `None` when both ends run the same build: then the builds are
/// not the cause.
pub fn refusal(server: Option<&str>, client: Option<&str>, detail: &str) -> Option<Text> {
    if server.is_some() && server == client {
        return None;
    }
    let english = Language::load(&[], crate::lang::DEFAULT);
    let translated = |key: &str, args: &[Text]| {
        Text::translate_with_fallback(key, english.get(key).unwrap(), args)
    };
    let named = |build: Option<&str>| match build {
        Some(build) => translated(
            "horizon.multiplayer.build_mismatch.build",
            &[Text::literal(build)],
        ),
        None => translated("horizon.multiplayer.build_mismatch.older", &[]),
    };
    let newer = match (server, client) {
        (None, _) => Some(Ordering::Less),
        (_, None) => Some(Ordering::Greater),
        (Some(server), Some(client)) => compare(server, client),
    };
    let advice = match newer {
        Some(Ordering::Greater) => "horizon.multiplayer.build_mismatch.update_game",
        Some(Ordering::Less) => "horizon.multiplayer.build_mismatch.update_server",
        _ => "horizon.multiplayer.build_mismatch.same_build",
    };
    Some(
        translated(KEY, &[named(server), named(client)])
            .append(" ")
            .append(translated(advice, &[]))
            .append("\n")
            .append(Text::literal(detail)),
    )
}

/// Which of two builds is newer, when their names say: two nightlies or snapshots by their date,
/// two releases by their version. A build from source names only its crate version, which no
/// release bumps, so it compares with nothing.
fn compare(a: &str, b: &str) -> Option<Ordering> {
    let order = match (key(a)?, key(b)?) {
        (Key::Dated(a), Key::Dated(b)) => a.cmp(b),
        (Key::Release(a), Key::Release(b)) => a.cmp(&b),
        _ => return None,
    };
    (order != Ordering::Equal).then_some(order)
}

enum Key<'a> {
    /// `Nightly 2026-10-28 · 0c02067`: the date.
    Dated(&'a str),
    /// `v0.2.3`, or a release from before 0.2 (`v0.1.9 (client 0.1.0)`): the numbers.
    Release(Vec<u64>),
}

fn key(build: &str) -> Option<Key<'_>> {
    let mut words = build.split_whitespace();
    let first = words.next()?;
    if matches!(first, "Nightly" | "Snapshot") {
        let date = words.next()?;
        let dated = date.len() == 10
            && date.bytes().enumerate().all(|(i, b)| {
                if i == 4 || i == 7 {
                    b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            });
        return dated.then_some(Key::Dated(date));
    }
    let numbers = first
        .strip_prefix('v')?
        .split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    Some(Key::Release(numbers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newer_build_is_told_by_date_or_release() {
        let nightly = |date: &str| format!("Nightly {date} · 0c02067");
        assert_eq!(
            compare(&nightly("2026-11-18"), &nightly("2026-11-16")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare("Snapshot 2026-11-13 · 1a2b3c4", &nightly("2026-11-16")),
            Some(Ordering::Less)
        );
        assert_eq!(compare("v0.2.10", "v0.2.9"), Some(Ordering::Greater));
        assert_eq!(
            compare("v0.1.9 (client 0.1.0)", "v0.2.0"),
            Some(Ordering::Less)
        );
        // Same day, a release against a nightly, or a build from source: nothing to tell.
        assert_eq!(
            compare(&nightly("2026-11-18"), "Nightly 2026-11-18 · 9f9f9f9"),
            None
        );
        assert_eq!(compare("v0.2.3", &nightly("2026-11-18")), None);
        assert_eq!(compare("0.2.0", "v0.2.3"), None);
    }
}
