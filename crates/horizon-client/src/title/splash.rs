//! The title screen's splash text (`SplashManager`), moved out of `title.rs` unchanged.
use horizon_core::random::RandomSource;

pub(crate) fn choose_splash(text: &str, index: usize, date: (u32, u32)) -> Option<String> {
    // SplashManager checks SpecialDates before the resource list (local date).
    let special = match date {
        (12, 24) => Some("Merry X-mas!"),
        (1, 1) => Some("Happy new year!"),
        (10, 31) => Some("OOoooOOOoooo! Spooky!"),
        _ => None,
    };
    if let Some(special) = special {
        return Some(special.to_owned());
    }
    let splashes: Vec<_> = splash_lines(text).collect();
    splashes
        .get(index.checked_rem(splashes.len())?)
        .map(|s| (*s).to_owned())
}
/// `SplashManager.prepare`: the splash list. Blank lines are kept; only the
/// hidden splash is dropped.
pub(super) fn splash_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|s| horizon_core::random::java_string_hash(s.encode_utf16()) != 125780783)
}

/// `SplashManager.getSplash`: outside the special dates, a one in `size` roll
/// (`nextInt(size) == 42`) shows the logged-in player's name instead of a splash.
pub(super) fn rolls_user_splash(
    text: &str,
    random: &mut impl RandomSource,
    date: (u32, u32),
) -> bool {
    if matches!(date, (12, 24) | (1, 1) | (10, 31)) {
        return false;
    }
    i32::try_from(splash_lines(text).count())
        .is_ok_and(|size| size > 0 && random.next_int_bound(size) == Ok(42))
}
