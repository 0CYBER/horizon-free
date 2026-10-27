//! Nightly (docs/release-flow.md "Nightly channel"): an opt-in build of main plus pull requests
//! picked by trusted reviewers, some not reviewed yet, published as the `nightly` pre-release each
//! night and when a reviewer asks. Off by default (`Settings::nightly`).
//!
//! It never touches the release: it installs as its own version, `nightly-<date>-<commit>`, and plays
//! in its own instance, "Nightly" (`instances/nightly/` in the game's home), so the player's worlds
//! stay where they are and choosing the Horizon instance plays the release again. Its checksums
//! must be signed for the nightly channel (`signed::nightly_digest`); the release channel refuses
//! them.
use crate::backend::{self, Release, ASSET};
use crate::instances::{self, Instance};
use horizon_release_sign::NIGHTLY;

/// The Nightly instance's id (its folder in `instances/`) and name.
pub const INSTANCE: &str = "nightly";
const NAME: &str = "Nightly";

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

/// The current nightly build, or None while there is none.
pub fn latest() -> Result<Option<Release>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{NIGHTLY}",
        backend::REPO
    );
    let release: serde_json::Value = match backend::get(&url) {
        Ok(response) => response.into_json().map_err(|e| e.to_string())?,
        Err(e) if e.ends_with("answered 404") => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(parse(&release))
}

fn parse(release: &serde_json::Value) -> Option<Release> {
    let list = serde_json::Value::Array(vec![release.clone()]);
    backend::parse_releases(&list, ASSET, |tag| tag == NIGHTLY)
        .into_iter()
        .next()
        .filter(|r| r.prerelease)
}

/// The nightly to install: `latest` when it isn't installed yet.
pub fn update(latest: Option<Release>, installed: &[String]) -> Option<Release> {
    latest.filter(|r| r.asset.is_some() && !installed.contains(&version_of(r)))
}

/// Installs `release` (the current nightly) as its own version, points the Nightly instance at
/// it (making the instance the first time) and removes older nightly versions. Returns the
/// instance.
pub fn install(release: &Release, progress: impl Fn(f32)) -> Result<Instance, String> {
    let sha256 = crate::signed::nightly_digest(
        ASSET.unwrap_or_default(),
        &horizon_release_sign::trusted_keys(),
        backend::fetch_small,
    )?;
    let version = version_of(release);
    backend::install_as(release, &version, &sha256, progress)?;
    let instance = instance_on(&version)?;
    for old in backend::installed() {
        if horizon_release_sign::is_nightly(&old) && old != version {
            let _ = backend::remove(&old);
        }
    }
    Ok(instance)
}

/// The Nightly instance, made empty the first time (with the player's options and offline name),
/// playing `version`.
fn instance_on(version: &str) -> Result<Instance, String> {
    if instances::find(INSTANCE).is_err() {
        instances::create(NAME, None)?;
    }
    instances::edit(INSTANCE, |meta| meta.version = Some(version.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(name: &str, date: &str, asset: bool) -> Release {
        Release {
            tag: NIGHTLY.into(),
            name: name.into(),
            body: String::new(),
            date: date.into(),
            asset: asset.then(|| ("https://example/horizon.zip".into(), 1)),
            prerelease: true,
        }
    }

    #[test]
    fn a_nightly_installs_under_its_date_and_commit() {
        let r = release("Nightly 2026-10-28 · 0c02067", "2026-10-28", true);
        assert_eq!(version_of(&r), "nightly-2026-10-28-0c02067");
        let untitled = release("Something else", "2026-10-29", true);
        assert_eq!(version_of(&untitled), "nightly-2026-10-29");
        // Never a release version: the release channel's checks and lists leave it out.
        assert!(!backend::is_game_tag(&version_of(&r)));
        assert!(horizon_release_sign::is_nightly(&version_of(&r)));
    }

    #[test]
    fn only_a_new_nightly_with_a_build_here_is_an_update() {
        let r = release("Nightly 2026-10-28 · 0c02067", "2026-10-28", true);
        let name = |r: Option<Release>| r.map(|r| r.name);
        assert_eq!(
            name(update(Some(r.clone()), &["v0.2.80".into()])),
            Some(r.name.clone())
        );
        let installed = ["nightly-2026-10-28-0c02067".to_string(), "v0.2.80".into()];
        assert!(update(Some(r), &installed).is_none());
        let no_build = release("Nightly 2026-10-29 · 1a2b3c4", "2026-10-29", false);
        assert!(update(Some(no_build), &installed).is_none());
        assert!(update(None, &installed).is_none());
    }

    #[test]
    fn only_the_nightly_pre_release_is_read() {
        let json = |tag: &str, pre: bool| {
            serde_json::json!({"tag_name": tag, "name": "Nightly 2026-10-28 · 0c02067",
                "prerelease": pre, "published_at": "2026-10-28T20:40:00Z",
                "assets": [{"name": ASSET.unwrap_or("none"), "browser_download_url": "u", "size": 1}]})
        };
        assert!(parse(&json("nightly", true)).is_some());
        assert!(parse(&json("nightly", false)).is_none());
        assert!(parse(&json("v0.2.81", true)).is_none());
    }
}
