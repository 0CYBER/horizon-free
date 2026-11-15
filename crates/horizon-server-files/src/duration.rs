//! Moderation durations: `/tempmute` and `/tempban`'s argument, and how long a mute or ban lasts when the
//! server tells it.

const MINUTE: i64 = 60_000;

/// The longest duration `parse_duration` accepts, in milliseconds (about 146 million years). Callers
/// add it to the clock (`now_millis() + ms`), so a value near `i64::MAX` would overflow that sum: a panic
/// in a debug build, an expiry in the past (a ban or mute that is already over) in a release build.
pub const MAX_DURATION: i64 = i64::MAX / 2;

/// `/tempmute`'s duration in milliseconds: a positive whole number and a unit, `s`, `m`, `h`, `d` or
/// `w` (`10m`, `2h`, `1d`), at most `MAX_DURATION`.
pub fn parse_duration(text: &str) -> Option<i64> {
    let unit = match text.chars().last()? {
        's' => 1_000,
        'm' => MINUTE,
        'h' => 60 * MINUTE,
        'd' => 24 * 60 * MINUTE,
        'w' => 7 * 24 * 60 * MINUTE,
        _ => return None,
    };
    let count: i64 = text[..text.len() - 1].parse().ok()?;
    (count > 0)
        .then(|| count.checked_mul(unit))
        .flatten()
        .filter(|&millis| millis <= MAX_DURATION)
}

/// The unit used when presenting a moderation duration to a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DurationUnit {
    Minute,
    Hour,
    Day,
}

/// A duration's rounded count and unit, shared by logs and translated replies.
pub fn duration_parts(millis: i64) -> (i64, DurationUnit) {
    let (unit, kind) = if millis <= 60 * MINUTE {
        (MINUTE, DurationUnit::Minute)
    } else if millis <= 48 * 60 * MINUTE {
        (60 * MINUTE, DurationUnit::Hour)
    } else {
        (24 * 60 * MINUTE, DurationUnit::Day)
    };
    (
        millis.div_euclid(unit) + i64::from(millis.rem_euclid(unit) > 0),
        kind,
    )
}

/// `millis` for logs, rounded up: minutes up to an hour, hours up to two days, then days.
pub fn duration_text(millis: i64) -> String {
    let (count, unit) = duration_parts(millis);
    let unit = match unit {
        DurationUnit::Minute => "minute",
        DurationUnit::Hour => "hour",
        DurationUnit::Day => "day",
    };
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_leaves_room_for_now() {
        // `106751991167d` is the largest day count whose milliseconds fit an `i64`: it passed `checked_mul`
        // and then overflowed `now_millis() + ms` in every caller.
        let day = 24 * 60 * MINUTE;
        assert_eq!(parse_duration("106751991167d"), None);
        assert_eq!(parse_duration("99999999999999999w"), None);
        // The cap itself: accepted values stay far enough from `i64::MAX` for any clock reading.
        let last_day = MAX_DURATION / day;
        let ms = parse_duration(&format!("{last_day}d")).expect("the last whole day under the cap");
        assert_eq!(ms, last_day * day);
        assert!(now_millis().checked_add(ms).is_some());
        assert_eq!(parse_duration(&format!("{}d", last_day + 1)), None);
        // Ordinary durations are unchanged.
        assert_eq!(parse_duration("10m"), Some(10 * MINUTE));
        assert_eq!(parse_duration("1w"), Some(7 * day));
    }
}
