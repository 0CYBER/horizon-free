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

/// When each Nightly the release lists read came out: its tag, and its publication date and
/// time (`HH:MM` UTC). A Nightly's tag keeps the day its cut started, which can be the day before
/// it came out, so every name of it reads its day from here ([`published`]).
static PUBLISHED: std::sync::RwLock<Vec<(String, String, String)>> =
    std::sync::RwLock::new(Vec::new());

/// Keeps, from the release list `releases`, when each Nightly came out, so every name of it says
/// that day, and its time when another came out the same day. A Nightly the list no longer has
/// (older than a week, say) keeps what it was seen with, and still counts toward its day.
pub fn note_published(releases: &[Release]) {
    let mut kept = PUBLISHED
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut known: Vec<(String, String, String)> = releases
        .iter()
        .filter(|r| r.nightly.is_some() && is_date(&r.date))
        .map(|r| (r.tag.clone(), r.date.clone(), r.time.clone()))
        .collect();
    for old in kept.iter() {
        if known.iter().all(|(t, ..)| *t != old.0) {
            known.push(old.clone());
        }
    }
    *kept = known;
}

/// When Nightly `version` came out, as its names say it: the day it was published (its tag's
/// day if no release list said), and the time (`HH:MM` UTC) when another came out that day, so
/// two never share a name. None for anything that is not a Nightly.
pub fn published(version: &str) -> Option<(String, Option<String>)> {
    let tag_date = date_of(version)?;
    let kept = PUBLISHED
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((_, date, time)) = kept.iter().find(|(tag, ..)| tag == version) else {
        return Some((tag_date.to_owned(), None));
    };
    let shared = kept.iter().filter(|(_, d, _)| d == date).count() > 1;
    Some((
        date.clone(),
        (shared && !time.is_empty()).then(|| time.clone()),
    ))
}

/// When Nightly `version` came out, `YYYY-MM-DD HH:MM` UTC (the day alone when the list gave no
/// time), if a release list said; for ordering two of one day.
pub fn published_at(version: &str) -> Option<String> {
    PUBLISHED
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|(tag, ..)| tag == version)
        .map(|(_, date, time)| format!("{date} {time}").trim_end().to_owned())
}

/// When Nightly `version` came out, `HH:MM` UTC, if another came out the same day.
pub fn same_day_time(version: &str) -> Option<String> {
    published(version)?.1
}

/// A Nightly as the Versions page names it, "the Nov 13 update" for `nightly-2026-11-13` (with
/// or without its commit), with its time when another came out that day ("the Nov 13, 07:22 UTC
/// update"); None for anything that is not one. The day is the one it came out ([`published`]).
pub fn update_name(version: &str) -> Option<String> {
    let (date, time) = published(version)?;
    Some(match time {
        Some(time) => crate::i18n::trf(
            "the {day}, {time} UTC update",
            &[("day", &day(&date)), ("time", &time)],
        ),
        None => crate::i18n::trf("the {day} update", &[("day", &day(&date))]),
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
    Some(match commit_of(version) {
        Some(commit) => format!("Nightly · {commit}"),
        None => "Nightly".to_owned(),
    })
}

/// A Nightly's commit, `4baa571` of `nightly-2026-10-29-4baa571`: what follows its date. None
/// for a Nightly without one, or anything that is not a Nightly.
pub fn commit_of(version: &str) -> Option<&str> {
    let date = date_of(version)?;
    version
        .strip_prefix("nightly-")?
        .strip_prefix(date)?
        .strip_prefix('-')
        .filter(|c| !c.is_empty())
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
        let _ = backend::remove(&old);
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

/// A build on the Stable channel; `week` is the Monday of its week when it stands for one.
#[derive(Debug, PartialEq, Eq)]
pub struct StableBuild {
    pub tag: String,
    pub week: Option<String>,
}

/// The last game release before Horizon shipped only Nightlies (docs/release-flow.md, "Nightly
/// channel"). It and every older tag stay full releases on GitHub, so `releases/latest` keeps
/// pointing at a game build for servers, but they are too old to be Stable.
const LAST_BEFORE_NIGHTLIES: &str = "v0.2.97";

/// The Stable channel's builds, newest first: the real (non-Nightly, non-pre-release) releases
/// after [`LAST_BEFORE_NIGHTLIES`] when any exist; until then the newest Nightly of each Monday to
/// Sunday week. `releases` is newest first, as GitHub lists them.
pub fn stable_builds(releases: &[Release]) -> Vec<StableBuild> {
    let real: Vec<_> = releases
        .iter()
        .filter(|r| {
            r.nightly.is_none()
                && r.asset.is_some()
                && !r.prerelease
                && backend::launcher_version(&r.tag).is_none()
                && backend::compare_versions(&r.tag, LAST_BEFORE_NIGHTLIES).is_gt()
        })
        .map(|r| StableBuild {
            tag: r.tag.clone(),
            week: None,
        })
        .collect();
    if !real.is_empty() {
        return real;
    }
    use chrono::Datelike;
    // (monday, date, tag): the newest date of a week wins; the first listed on a tie.
    let mut weeks: Vec<(String, String, String)> = Vec::new();
    for r in releases
        .iter()
        .filter(|r| r.nightly.is_some() && r.asset.is_some())
    {
        let Some(date) = date_of(&r.tag) else {
            continue;
        };
        let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            continue;
        };
        let monday =
            (day - chrono::Duration::days(day.weekday().num_days_from_monday() as i64)).to_string();
        match weeks.iter_mut().find(|w| w.0 == monday) {
            Some(w) if w.1.as_str() < date => *w = (monday, date.into(), r.tag.clone()),
            Some(_) => {}
            None => weeks.push((monday, date.into(), r.tag.clone())),
        }
    }
    weeks.sort_by(|a, b| b.0.cmp(&a.0));
    weeks
        .into_iter()
        .map(|(monday, _, tag)| StableBuild {
            tag,
            week: Some(monday),
        })
        .collect()
}

/// The build Stable offers in `releases`: the newest of [`stable_builds`].
pub fn stable_target(releases: &[Release]) -> Option<Release> {
    let tag = stable_builds(releases).into_iter().next()?.tag;
    releases.iter().find(|r| r.tag == tag).cloned()
}

/// Whether `a` and `b` are the same download: one tag, title and file.
fn same_build(a: &Release, b: &Release) -> bool {
    a.tag == b.tag && a.name == b.name && a.asset == b.asset
}

/// What Home offers a Stable instance set to Latest: [`stable_target`] when it isn't installed,
/// and isn't `failed`, a download that failed and stays left out until the list is read again
/// from GitHub.
pub fn stable_offer(
    releases: &[Release],
    installed: &[String],
    failed: Option<&Release>,
) -> Option<Release> {
    stable_target(releases)
        .filter(|next| !installed.contains(&next.tag))
        .filter(|next| !failed.is_some_and(|f| same_build(f, next)))
}

/// Installs `target`, Home's Stable update, in one press. GitHub's `nightly` may have moved on
/// since the offer was read, so the download fails its check: then the list is read again from
/// the network (`fresh`), given to `read` with the new target, and that one is installed once,
/// unless it is `installed` already (by the command line, say): then nothing is downloaded or
/// replaced. Returns the build tried last and how it went.
pub fn install_stable(
    target: Release,
    fresh: impl FnOnce() -> Result<Vec<Release>, String>,
    read: impl FnOnce(Vec<Release>, Option<&Release>),
    installed: impl FnOnce(&str) -> bool,
    install: impl Fn(&Release) -> Result<(), String>,
) -> (Release, Result<(), String>) {
    let Err(error) = install(&target) else {
        return (target, Ok(()));
    };
    let Ok(list) = fresh() else {
        return (target, Err(error));
    };
    let next = stable_target(&list).filter(|n| !same_build(n, &target));
    read(list, next.as_ref());
    match next {
        Some(next) if installed(&next.tag) => (next, Ok(())),
        Some(next) => {
            let result = install(&next);
            (next, result)
        }
        None => (target, Err(error)),
    }
}

/// What a Stable instance set to Latest plays, from the installed builds alone (`selected`: the
/// launcher's version): the newest release after [`LAST_BEFORE_NIGHTLIES`]; else the launcher's
/// version, unless it is a release from before the Nightlies; else the newest Nightly. None when
/// there is none of these: the caller falls back to the launcher's version.
pub fn stable_latest(installed: &[String], selected: Option<&str>) -> Option<String> {
    let newest = |keep: &dyn Fn(&str) -> bool| {
        installed
            .iter()
            .filter(|t| keep(t))
            // The first listed wins a tie (two builds of one day).
            .reduce(|a, b| match backend::compare_versions(b, a).is_gt() {
                true => b,
                false => a,
            })
            .cloned()
    };
    let before_nightlies = |tag: &str| {
        backend::is_game_tag(tag) && backend::compare_versions(tag, LAST_BEFORE_NIGHTLIES).is_le()
    };
    newest(&|t| backend::is_game_tag(t) && !t.contains('-') && !before_nightlies(t))
        .or_else(|| {
            selected
                .filter(|s| installed.iter().any(|t| t == s) && !before_nightlies(s))
                .map(str::to_owned)
        })
        .or_else(|| newest(&|t| date_of(t).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(tag: &str, nightly: bool, asset: bool) -> Release {
        Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            asset: asset.then(|| ("horizon.tar.gz".into(), 1)),
            prerelease: false,
            nightly: nightly.then(|| "nightly".into()),
        }
    }

    #[test]
    fn stable_is_a_build_a_week_until_real_releases_exist() {
        // 2026-11-09 is a Monday, 2026-11-08 the Sunday before it.
        let mut releases = vec![
            build("nightly-2026-11-13-aaaaaaa", true, true),
            build("nightly-2026-11-12-bbbbbbb", true, true),
            build("nightly-2026-11-10-ccccccc", true, true),
            build("nightly-2026-11-08-ddddddd", true, true),
            build("nightly-2026-11-03-eeeeeee", true, true),
            // No build for this platform: not a candidate.
            build("nightly-2026-11-14-fffffff", true, false),
            // The releases from before the Nightlies are full releases, but not Stable.
            build("v0.2.97", false, true),
            build("v0.2.97.1", false, true),
            build("v0.2.96", false, true),
            build("v2026.10.19.15", false, true),
        ];
        let weekly = |tag: &str, week: &str| StableBuild {
            tag: tag.into(),
            week: Some(week.into()),
        };
        assert_eq!(
            stable_builds(&releases),
            [
                weekly("nightly-2026-11-13-aaaaaaa", "2026-11-09"),
                weekly("nightly-2026-11-08-ddddddd", "2026-11-02"),
            ]
        );
        releases.insert(0, build("v0.3.0", false, true));
        assert_eq!(
            stable_builds(&releases),
            [StableBuild {
                tag: "v0.3.0".into(),
                week: None
            }]
        );
    }

    #[test]
    fn a_stable_install_that_fails_its_check_reads_the_list_again_and_retries_once() {
        let old = build("nightly-2026-11-13-aaaaaaa", true, true);
        let new = build("nightly-2026-11-13-bbbbbbb", true, true);
        let tries = std::cell::RefCell::new(Vec::new());
        let install = |fails: &'static [&'static str]| {
            let tries = &tries;
            move |r: &Release| {
                tries.borrow_mut().push(r.tag.clone());
                match fails.contains(&r.tag.as_str()) {
                    true => Err(format!("{}: checksum mismatch", r.tag)),
                    false => Ok(()),
                }
            }
        };
        // `nightly` moved on: the fresh list's target is installed, in the same press.
        let mut read = None;
        let (done, result) = install_stable(
            old.clone(),
            || Ok(vec![new.clone()]),
            |_, next| read = next.map(|n| n.tag.clone()),
            |_| false,
            install(&["nightly-2026-11-13-aaaaaaa"]),
        );
        assert_eq!((done.tag.as_str(), result), (new.tag.as_str(), Ok(())));
        assert_eq!(read.as_deref(), Some(new.tag.as_str()));
        assert_eq!(*tries.borrow(), [old.tag.clone(), new.tag.clone()]);
        // The new target already installed (by the command line, say): nothing downloaded.
        tries.borrow_mut().clear();
        let mut read = None;
        let (done, result) = install_stable(
            old.clone(),
            || Ok(vec![new.clone()]),
            |_, next| read = next.map(|n| n.tag.clone()),
            |tag| tag == new.tag,
            install(&["nightly-2026-11-13-aaaaaaa"]),
        );
        assert_eq!((done.tag.as_str(), result), (new.tag.as_str(), Ok(())));
        assert_eq!(read.as_deref(), Some(new.tag.as_str()));
        assert_eq!(*tries.borrow(), std::slice::from_ref(&old.tag));
        // Only once: the retry failing too is the error shown.
        tries.borrow_mut().clear();
        let (done, result) = install_stable(
            old.clone(),
            || Ok(vec![new.clone()]),
            |_, _| {},
            |_| false,
            install(&["nightly-2026-11-13-aaaaaaa", "nightly-2026-11-13-bbbbbbb"]),
        );
        assert_eq!(done.tag, new.tag);
        assert!(result.is_err());
        assert_eq!(tries.borrow().len(), 2);
        // The list still offers the same build, or can't be read: no retry.
        for fresh in [Ok(vec![old.clone()]), Err("offline".to_string())] {
            tries.borrow_mut().clear();
            let (done, result) = install_stable(
                old.clone(),
                || fresh,
                |_, _| {},
                |_| false,
                install(&["nightly-2026-11-13-aaaaaaa"]),
            );
            assert_eq!(done.tag, old.tag);
            assert!(result.is_err());
            assert_eq!(tries.borrow().len(), 1);
        }
    }

    #[test]
    fn a_failed_stable_download_is_not_offered_again() {
        let target = build("nightly-2026-11-13-aaaaaaa", true, true);
        let list = [target.clone(), build("v0.2.97", false, true)];
        let offer = |installed: &[String], failed: Option<&Release>| {
            stable_offer(&list, installed, failed).map(|r| r.tag)
        };
        assert_eq!(offer(&[], None), Some(target.tag.clone()));
        assert_eq!(offer(&[], Some(&target)), None);
        // Another build of the same tag (a new title or file) is offered.
        let mut moved = target.clone();
        moved.asset = Some(("other.tar.gz".into(), 2));
        assert_eq!(offer(&[], Some(&moved)), Some(target.tag.clone()));
        // Installed: nothing to offer.
        assert_eq!(offer(std::slice::from_ref(&target.tag), None), None);
    }

    #[test]
    fn stable_on_latest_plays_an_installed_build_never_a_pre_nightly_release() {
        let installed = |tags: &[&str]| tags.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        let nightlies = [
            "nightly-2026-11-13-aaaaaaa",
            "nightly-2026-11-08-ddddddd",
            "v0.2.97",
            "v0.2.97.1",
        ];
        // The release from before the Nightlies, selected: the newest Nightly instead.
        assert_eq!(
            stable_latest(&installed(&nightlies), Some("v0.2.97")).as_deref(),
            Some("nightly-2026-11-13-aaaaaaa")
        );
        assert_eq!(
            stable_latest(&installed(&nightlies), Some("v0.2.97.1")).as_deref(),
            Some("nightly-2026-11-13-aaaaaaa")
        );
        // A release after them wins.
        let with_release = [&["v0.3.0"], &nightlies[..]].concat();
        assert_eq!(
            stable_latest(
                &installed(&with_release),
                Some("nightly-2026-11-13-aaaaaaa")
            )
            .as_deref(),
            Some("v0.3.0")
        );
        // An older Nightly picked by hand stays picked.
        assert_eq!(
            stable_latest(&installed(&nightlies), Some("nightly-2026-11-08-ddddddd")).as_deref(),
            Some("nightly-2026-11-08-ddddddd")
        );
        // Only the old release: none, so the caller's fallback (the launcher's version) plays.
        assert_eq!(
            stable_latest(&installed(&["v0.2.97"]), Some("v0.2.97")),
            None
        );
    }

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
    fn a_nightly_is_named_by_when_it_came_out_and_reads_its_commit_after_the_date() {
        let nightly = |version: &str, date: &str, time: &str| Release {
            tag: version.into(),
            time: time.into(),
            nightly: Some(NIGHTLY.into()),
            ..release("", date)
        };
        // The 2026-11-19 cut of 5ef7734 came out on 11-20: its tag's day and its publication
        // day differ.
        note_published(&[
            nightly("nightly-2026-11-20-0a875da", "2026-11-20", "11:18"),
            nightly("nightly-2026-11-19-5ef7734", "2026-11-20", "02:32"),
            nightly("nightly-2026-11-19-f00125f", "2026-11-19", "15:33"),
            nightly("nightly-2026-11-18-dd67b76", "2026-11-18", "09:41"),
        ]);
        // The launcher's language may not be English: build the names it should say.
        let name = |v: &str| update_name(v).unwrap();
        let at = |date: &str, time: &str| {
            crate::i18n::trf(
                "the {day}, {time} UTC update",
                &[("day", &day(date)), ("time", time)],
            )
        };
        let on = |date: &str| crate::i18n::trf("the {day} update", &[("day", &day(date))]);
        assert_eq!(
            name("nightly-2026-11-20-0a875da"),
            at("2026-11-20", "11:18")
        );
        assert_eq!(
            name("nightly-2026-11-19-5ef7734"),
            at("2026-11-20", "02:32")
        );
        // Alone on the day it came out, a Nightly is named by that day alone, even if another
        // shares its tag's day.
        assert_eq!(name("nightly-2026-11-19-f00125f"), on("2026-11-19"));
        assert_eq!(name("nightly-2026-11-18-dd67b76"), on("2026-11-18"));
        assert_eq!(same_day_time("nightly-2026-11-19-f00125f"), None);
        assert_eq!(
            published("nightly-2026-11-19-5ef7734"),
            Some(("2026-11-20".into(), Some("02:32".into())))
        );
        // A Nightly no list said anything about keeps its tag's day.
        assert_eq!(
            published("nightly-2026-11-17-1234567"),
            Some(("2026-11-17".into(), None))
        );
        for (tag, commit) in [
            ("nightly-2026-11-20-0a875da", "0a875da"),
            ("nightly-2026-11-19-5ef7734", "5ef7734"),
            ("nightly-2026-11-19-f00125f", "f00125f"),
            ("nightly-2026-11-18-dd67b76", "dd67b76"),
        ] {
            assert_eq!(commit_of(tag), Some(commit));
            assert_eq!(label(tag), Some(format!("Nightly · {commit}")));
        }
        assert_eq!(commit_of("nightly-2026-11-18"), None);
        assert_eq!(commit_of("nightly-2026-11-18-"), None);
        assert_eq!(commit_of("v0.2.97"), None);
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
