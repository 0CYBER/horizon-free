//! Nightly (docs/release-flow.md): the only build Horizon ships since 2026-10-28, main plus the pull
//! requests the trusted reviewers picked, published as the `nightly` pre-release a few times a day.
//! Every launcher installs it as its update (`backend::horizon_update`), as its own version
//! `nightly-<date>-<commit>`, beside the player's worlds, which it never touches. Its checksums must
//! be signed for the nightly channel (`signed::build_digest`).
//!
//! Before that it was opt-in and played in its own "Nightly" instance (`instances/nightly/`):
//! that instance, and its worlds, stay, and keep following the Nightly.
use crate::backend::{self, Release};
use crate::instances;
use horizon_release_sign::NIGHTLY;

/// The id of the instance (its folder in `instances/`) the opt-in Nightly played in.
pub const INSTANCE: &str = "nightly";

/// The version a nightly installs as: `nightly-2026-10-28-0c02067`, from its title
/// ("Nightly 2026-10-28 · 0c02067"), else its publication date.
pub fn version_of(release: &Release) -> String {
    let mut words = release
        .name
        .strip_prefix("Nightly ")
        .unwrap_or_default()
        .split(" · ");
    let date = words.next().filter(|d| is_date(d)).unwrap_or(&release.date);
    let commit = words
        .next()
        .filter(|c| !c.is_empty() && c.chars().all(|c| c.is_ascii_hexdigit()));
    match commit {
        Some(commit) => format!("{NIGHTLY}-{date}-{commit}"),
        None => format!("{NIGHTLY}-{date}"),
    }
}

/// The date of a nightly's version (`nightly-2026-10-28-0c02067`), or None for any other version.
pub fn date_of(version: &str) -> Option<&str> {
    let date = version.strip_prefix("nightly-")?.get(..10)?;
    is_date(date).then_some(date)
}

/// A day as players read it, "Nov 1", from `2026-11-01`; anything else as it is.
pub fn day(date: &str) -> String {
    use crate::i18n::trf;
    let day = date
        .get(8..10)
        .map(|d| d.trim_start_matches('0').to_string())
        .unwrap_or_default();
    let args = [("day", day.as_str())];
    match date.get(5..7) {
        Some("01") => trf("Jan {day}", &args),
        Some("02") => trf("Feb {day}", &args),
        Some("03") => trf("Mar {day}", &args),
        Some("04") => trf("Apr {day}", &args),
        Some("05") => trf("May {day}", &args),
        Some("06") => trf("Jun {day}", &args),
        Some("07") => trf("Jul {day}", &args),
        Some("08") => trf("Aug {day}", &args),
        Some("09") => trf("Sep {day}", &args),
        Some("10") => trf("Oct {day}", &args),
        Some("11") => trf("Nov {day}", &args),
        Some("12") => trf("Dec {day}", &args),
        _ => date.to_string(),
    }
}

/// The Nightlies of the last release list read that share their day with another, and the time
/// (`HH:MM` UTC) each came out: [`same_day_time`] reads it for every screen that names a version.
static SAME_DAY: std::sync::RwLock<Vec<(String, String)>> = std::sync::RwLock::new(Vec::new());

/// Keeps, from the release list `releases`, when each Nightly that shares its day with another
/// came out, so every name of it says its time. A Nightly the list no longer has (older than a
/// week, say) keeps the time it was seen with, and still counts toward its day.
pub fn note_published(releases: &[Release]) {
    let mut kept = SAME_DAY
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut known: Vec<(&str, &str, &str)> = releases
        .iter()
        .filter(|r| r.nightly.is_some())
        .map(|r| (r.tag.as_str(), r.date.as_str(), r.time.as_str()))
        .collect();
    for (tag, time) in kept.iter() {
        if let Some(date) = date_of(tag).filter(|_| known.iter().all(|(t, ..)| t != tag)) {
            known.push((tag, date, time));
        }
    }
    let same_day = known
        .iter()
        .filter(|(_, date, time)| {
            !time.is_empty() && known.iter().filter(|(_, d, _)| d == date).count() > 1
        })
        .map(|(tag, _, time)| (tag.to_string(), time.to_string()))
        .collect();
    *kept = same_day;
}

/// When Nightly `version` came out, `HH:MM` UTC, if another came out the same day.
pub fn same_day_time(version: &str) -> Option<String> {
    SAME_DAY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|(tag, _)| tag == version)
        .map(|(_, time)| time.clone())
}

/// A Nightly as the Versions page names it, "the Nov 13 update" for `nightly-2026-11-13` (with
/// or without its commit), with its time when another came out that day ("the Nov 13, 07:22 UTC
/// update"); None for anything that is not one.
pub fn update_name(version: &str) -> Option<String> {
    let date = date_of(version)?;
    Some(match same_day_time(version) {
        Some(time) => crate::i18n::trf(
            "the {day}, {time} UTC update",
            &[("day", &day(date)), ("time", &time)],
        ),
        None => crate::i18n::trf("the {day} update", &[("day", &day(date))]),
    })
}

/// A full 40-character commit as a player reads it, `b1ff51d9e…`; anything else as it is.
pub fn short_commit(version: &str) -> String {
    if version.len() == 40 && version.bytes().all(|b| b.is_ascii_hexdigit()) {
        format!("{}…", &version[..9])
    } else {
        version.to_owned()
    }
}

/// A Nightly's version as a player reads it: `Nightly · 4baa571` for `nightly-2026-10-29-4baa571`,
/// or None for anything that is not one. The date is already on the row beside it.
pub fn label(version: &str) -> Option<String> {
    if let Some(label) = crate::developer_install::label(version) {
        return Some(label);
    }
    date_of(version)?;
    let commit = version.strip_prefix("nightly-")?.get(11..);
    Some(match commit {
        Some(commit) if !commit.is_empty() => format!("Nightly · {commit}"),
        _ => "Nightly".to_owned(),
    })
}

fn is_date(text: &str) -> bool {
    text.len() == 10
        && text.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// After `version`, a Nightly, installed as an update: the old Nightly instance follows it, and the
/// older Nightlies go unless the selection or an instance still plays them. Worlds stay.
pub fn after_update(version: &str) {
    let list = instances::list();
    if list
        .iter()
        .any(|i| i.id == INSTANCE && i.version.as_deref().is_some_and(|v| date_of(v).is_some()))
    {
        let _ = instances::edit(INSTANCE, |meta| meta.version = Some(version.to_owned()));
    }
    let selected = backend::Settings::load().selected;
    let pinned: Vec<String> = instances::list()
        .into_iter()
        .filter_map(|i| i.version)
        .collect();
    for old in stale(
        &backend::installed(),
        version,
        selected.as_deref(),
        &pinned,
        backend::auto_installed,
    ) {
        // A removal stops at the first running program, with half the version already gone.
        if !backend::in_use(&old) {
            let _ = backend::remove(&old);
        }
    }
}

/// The automatically installed Nightlies to remove once `current` is installed: every other one
/// that is neither selected nor pinned by an instance. Manual and legacy installs stay.
fn stale(
    installed: &[String],
    current: &str,
    selected: Option<&str>,
    pinned: &[String],
    auto: impl Fn(&str) -> bool,
) -> Vec<String> {
    installed
        .iter()
        .filter(|v| {
            date_of(v).is_some() && auto(v) && *v != current && Some(v.as_str()) != selected
        })
        .filter(|v| !pinned.contains(v))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(name: &str, date: &str) -> Release {
        Release {
            tag: NIGHTLY.into(),
            name: name.into(),
            body: String::new(),
            date: date.into(),
            time: String::new(),
            asset: Some(("https://example/horizon.zip".into(), 1)),
            prerelease: true,
            nightly: None,
        }
    }

    #[test]
    fn a_nightly_installs_under_its_date_and_commit() {
        let r = release("Nightly 2026-10-28 · 0c02067", "2026-10-28");
        assert_eq!(version_of(&r), "nightly-2026-10-28-0c02067");
        let untitled = release("Something else", "2026-10-29");
        assert_eq!(version_of(&untitled), "nightly-2026-10-29");
        assert_eq!(date_of(&version_of(&r)), Some("2026-10-28"));
        assert_eq!(date_of("v0.2.95"), None);
        assert_eq!(date_of("nightly"), None);
        assert!(horizon_release_sign::is_nightly(&version_of(&r)));
    }

    #[test]
    fn two_nightlies_of_one_day_are_named_with_their_times() {
        let nightly = |version: &str, date: &str, time: &str| Release {
            tag: version.into(),
            time: time.into(),
            nightly: Some(NIGHTLY.into()),
            ..release("", date)
        };
        note_published(&[
            nightly("nightly-2031-03-04-bbbbbbb", "2031-03-04", "18:05"),
            nightly("nightly-2031-03-04-aaaaaaa", "2031-03-04", "07:22"),
            nightly("nightly-2031-03-03-ccccccc", "2031-03-03", "09:00"),
        ]);
        let late = update_name("nightly-2031-03-04-bbbbbbb").unwrap();
        let early = update_name("nightly-2031-03-04-aaaaaaa").unwrap();
        assert_ne!(late, early);
        assert!(
            late.contains("18:05") && early.contains("07:22"),
            "{late} / {early}"
        );
        // Alone on its day, a Nightly keeps its day's name.
        assert_eq!(
            update_name("nightly-2031-03-03-ccccccc"),
            update_name("nightly-2031-03-03")
        );
    }

    #[test]
    fn a_nightly_the_list_no_longer_has_keeps_its_time() {
        let nightly = |version: &str, time: &str| Release {
            tag: version.into(),
            time: time.into(),
            nightly: Some(NIGHTLY.into()),
            ..release("", "2032-05-06")
        };
        let late = nightly("nightly-2032-05-06-bbbbbbb", "18:05");
        note_published(&[late.clone(), nightly("nightly-2032-05-06-aaaaaaa", "07:22")]);
        // A later list without the earlier build: both still read apart.
        note_published(&[late]);
        assert_eq!(
            same_day_time("nightly-2032-05-06-aaaaaaa").as_deref(),
            Some("07:22")
        );
        assert_eq!(
            same_day_time("nightly-2032-05-06-bbbbbbb").as_deref(),
            Some("18:05")
        );
    }

    #[test]
    fn a_full_commit_reads_as_nine_characters() {
        let full = "b1ff51d9e0123456789abcdef0123456789abcde";
        assert_eq!(short_commit(full), "b1ff51d9e…");
        assert_eq!(short_commit("v0.2.97"), "v0.2.97");
    }

    #[test]
    fn a_nightly_reads_as_its_commit() {
        assert_eq!(
            label("nightly-2026-10-29-4baa571").as_deref(),
            Some("Nightly · 4baa571")
        );
        assert_eq!(label("nightly-2026-10-29").as_deref(), Some("Nightly"));
        assert_eq!(label("v0.2.97"), None);
    }

    #[test]
    fn only_unused_older_nightlies_are_removed() {
        let installed = [
            "nightly-2026-10-29-aaaaaaa",
            "nightly-2026-10-28-bbbbbbb",
            "nightly-2026-10-28-ccccccc",
            "nightly-2026-10-27-ddddddd",
            "v0.2.95",
        ]
        .map(String::from);
        let pinned = ["nightly-2026-10-27-ddddddd".to_string()];
        assert_eq!(
            stale(
                &installed,
                "nightly-2026-10-29-aaaaaaa",
                Some("nightly-2026-10-28-ccccccc"),
                &pinned,
                |tag| tag != "nightly-2026-10-28-ccccccc"
            ),
            ["nightly-2026-10-28-bbbbbbb"]
        );
    }

    #[test]
    fn cli_update_keeps_unselected_manual_nightly() {
        let current = "nightly-2026-11-05-aaaaaaa";
        let manual = "nightly-2026-11-03-bbbbbbb";
        let previous_auto = "nightly-2026-11-02-ccccccc";
        let installed = [current, manual, previous_auto].map(String::from);
        assert_eq!(
            stale(&installed, current, None, &[], |tag| tag != manual),
            [previous_auto]
        );
    }
}
