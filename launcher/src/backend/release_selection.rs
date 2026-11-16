//! Resolve a published release's channel or exact installed version.
use super::Release;
use horizon_release_sign::Channel;

/// The snapshot tagged `tag` (`snapshot-<date>-<commit>`, docs/release-flow.md "Snapshot"), looked
/// up by its tag alone, or nothing for any other tag. No list or update ever offers a snapshot, so a
/// developer names it (`horizon-launcher install <tag>`); it installs as its tag, which its title
/// must name and its SHA256SUMS.v2.sig must be signed for (`backend::install`).
pub(super) fn snapshot(tag: &str) -> Result<Vec<Release>, String> {
    if Channel::of(tag) != Some(Channel::Snapshot) {
        return Ok(Vec::new());
    }
    if !horizon_release_sign::is_build(Channel::Snapshot, tag) {
        return Err(format!(
            "{tag} is not a snapshot's snapshot-<date>-<commit>."
        ));
    }
    let release = crate::github::json_optional(&format!(
        "{}/repos/{}/releases/tags/{tag}",
        crate::github::API,
        super::REPO
    ))?
    .ok_or_else(|| format!("No Horizon snapshot is tagged {tag}."))?;
    let settings = super::Settings::load();
    let assets = super::game_assets_for(
        std::env::consts::OS,
        super::native_windows_architecture(),
        std::env::consts::ARCH,
        settings.game_architecture,
    );
    let (asset, fallback) = assets
        .map_or((super::ASSET, super::GAME_FALLBACK), |(asset, fallback)| {
            (Some(asset), fallback)
        });
    Ok(
        snapshot_in(&serde_json::json!([release]), tag, asset, fallback)
            .into_iter()
            .collect(),
    )
}

/// The snapshot tagged `tag` in `list` (GitHub's answer for it) with `asset`, else `fallback`.
fn snapshot_in(
    list: &serde_json::Value,
    tag: &str,
    asset: Option<&str>,
    fallback: Option<&str>,
) -> Option<Release> {
    let keep = |tag: &str| Channel::of(tag) == Some(Channel::Snapshot);
    let mut found = super::parse_releases(list, asset, keep)
        .into_iter()
        .chain(super::parse_releases(list, fallback, keep))
        .filter(|r| r.tag == tag);
    let mut snapshot = found.next()?;
    if snapshot.asset.is_none() {
        snapshot.asset = found.find_map(|r| r.asset);
    }
    // Its GitHub tag is its version: its files are there, and it installs and verifies as that.
    snapshot.prerelease = true;
    snapshot.nightly = Some(snapshot.tag.clone());
    Some(snapshot)
}

pub(super) fn by_tag<'a>(releases: &'a [Release], tag: &str) -> Option<&'a Release> {
    releases
        .iter()
        .find(|release| release.tag == tag || release.nightly.as_deref() == Some(tag))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::game_releases_with;

    fn published(channel: &str) -> Vec<Release> {
        game_releases_with(
            &serde_json::json!([{
                "tag_name": channel,
                "name": "Nightly 2026-11-10 · 9f13812",
                "published_at": "2026-11-10T17:07:46Z",
                "prerelease": true,
                "assets": [{"name": "horizon-linux-x86_64.tar.gz", "size": 123}]
            }]),
            Some("horizon-linux-x86_64.tar.gz"),
            None,
        )
    }

    #[test]
    fn channel_and_exact_version_select_the_same_published_build() {
        for channel in ["nightly", "nightly-2026-11-10"] {
            let releases = published(channel);
            for requested in [channel, "nightly-2026-11-10-9f13812"] {
                let release = by_tag(&releases, requested).unwrap();
                assert_eq!(release.tag, "nightly-2026-11-10-9f13812");
                assert_eq!(release.nightly.as_deref(), Some(channel));
                assert_eq!(
                    release.asset,
                    Some((format!("https://github.com/horizon-rust/horizon/releases/download/{channel}/horizon-linux-x86_64.tar.gz"), 123))
                );
            }
        }
    }

    #[test]
    fn a_snapshot_is_found_by_its_tag_and_installs_as_it() {
        const SNAPSHOT: &str = "snapshot-2026-11-13-bbbbbbb";
        let (arm, x64) = ("horizon-windows-arm64.zip", "horizon-windows-x64.zip");
        let list = serde_json::json!([{
            "tag_name": SNAPSHOT,
            "name": "Snapshot 2026-11-13 · bbbbbbb",
            "published_at": "2026-11-13T10:00:00Z",
            "prerelease": true,
            "assets": [{"name": x64, "size": 1}]
        }]);
        let found = snapshot_in(&list, SNAPSHOT, Some(arm), Some(x64)).unwrap();
        assert_eq!(found.tag, SNAPSHOT);
        assert_eq!(found.nightly.as_deref(), Some(SNAPSHOT));
        assert!(found.prerelease);
        assert!(found
            .asset
            .unwrap()
            .0
            .ends_with(&format!("/{SNAPSHOT}/{x64}")));
        assert_eq!(
            crate::signed::title_version(SNAPSHOT, &found.name).as_deref(),
            Some(SNAPSHOT),
            "what install() checks before verifying"
        );
        assert!(snapshot_in(&list, "snapshot-2026-11-12-aaaaaaa", Some(x64), None).is_none());
        let nightly = serde_json::json!([{"tag_name": "nightly", "name": "Nightly 2026-11-13 · bbbbbbb",
                                           "assets": [{"name": x64, "size": 1}]}]);
        assert!(snapshot_in(&nightly, "nightly", Some(x64), None).is_none());
        // Any other tag is not looked up; a malformed snapshot tag is refused before any request.
        assert_eq!(snapshot("nightly").unwrap().len(), 0);
        assert_eq!(snapshot("v0.2.81").unwrap().len(), 0);
        assert!(snapshot("snapshot-../../x").is_err());
    }

    #[test]
    fn an_unknown_tag_never_falls_back_to_an_available_build() {
        let releases = published("nightly");
        assert!(by_tag(&releases, "nightly-2026-11-09").is_none());
        assert!(by_tag(&releases, "nightly-2026-11-10-deadbee").is_none());
        assert!(by_tag(&[], "nightly").is_none());
    }
}
