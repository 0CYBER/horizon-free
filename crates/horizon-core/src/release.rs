//! Which Horizon release this build is (docs/versioning.md). Not Minecraft's version: that is
//! [`crate::version`], generated from the game's own files.

/// The release's version (`0.2.3`, no `v`): release.yml builds every release with `HORIZON_VERSION`
/// set from its tag, so there are no version-bump commits. None for a build from source.
pub const RELEASE: Option<&str> = match option_env!("HORIZON_VERSION") {
    Some(version) if !version.is_empty() => Some(version),
    _ => None,
};

/// [`RELEASE`], or this crate's Cargo.toml version for a build from source: what `--version`, user
/// agents and reports say.
pub const VERSION: &str = match RELEASE {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// A nightly's date and commit (`2026-10-28 0c02067`): nightly-build.yml builds the opt-in Nightly
/// channel with `HORIZON_NIGHTLY` set (docs/release-flow.md), and a snapshot the same way, with
/// `HORIZON_VERSION` `snapshot-<date>` ([`kind`]). None for a release or a build from source.
pub const NIGHTLY: Option<&str> = match option_env!("HORIZON_NIGHTLY") {
    Some(nightly) if !nightly.is_empty() => Some(nightly),
    _ => None,
};

/// How a nightly or a snapshot names itself where players read it, so their reports say which
/// build they come from: `Nightly 2026-10-28 · 0c02067`, `Snapshot 2026-11-13 · 0c02067`. None for
/// any other build.
pub fn nightly_label() -> Option<String> {
    NIGHTLY.map(|nightly| label(kind(RELEASE), nightly))
}

/// A nightly's or snapshot's version as the launcher installs it (`nightly-2026-10-28-0c02067`,
/// `snapshot-2026-11-13-0c02067`).
pub fn nightly_version() -> Option<String> {
    NIGHTLY.map(|n| {
        format!(
            "{}-{}",
            kind(RELEASE).to_ascii_lowercase(),
            n.replace(' ', "-")
        )
    })
}

/// Which build of nightly-build.yml this is: `Snapshot` when it was built as one (release.yml kind
/// snapshot sets `HORIZON_VERSION` to `snapshot-<date>`), else `Nightly`.
fn kind(release: Option<&str>) -> &'static str {
    if release.is_some_and(|version| version.starts_with("snapshot-")) {
        "Snapshot"
    } else {
        "Nightly"
    }
}

fn label(kind: &str, nightly: &str) -> String {
    match nightly.split_once(' ') {
        Some((date, commit)) => format!("{kind} {date} · {commit}"),
        None => format!("{kind} {nightly}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_nightly_names_its_date_and_commit() {
        assert_eq!(
            super::label("Nightly", "2026-10-28 0c02067"),
            "Nightly 2026-10-28 · 0c02067"
        );
        assert_eq!(super::label("Nightly", "2026-10-28"), "Nightly 2026-10-28");
        assert_eq!(super::nightly_label().is_some(), super::NIGHTLY.is_some());
    }

    #[test]
    fn a_snapshot_names_itself_a_snapshot() {
        assert_eq!(super::kind(Some("snapshot-2026-11-13")), "Snapshot");
        assert_eq!(super::kind(Some("nightly-2026-11-13")), "Nightly");
        assert_eq!(super::kind(None), "Nightly");
        assert_eq!(
            super::label(
                super::kind(Some("snapshot-2026-11-13")),
                "2026-11-13 0c02067"
            ),
            "Snapshot 2026-11-13 · 0c02067"
        );
    }

    #[test]
    fn the_version_is_the_release_or_the_crate() {
        assert_eq!(
            super::RELEASE,
            option_env!("HORIZON_VERSION").filter(|v| !v.is_empty())
        );
        assert_eq!(
            super::VERSION,
            super::RELEASE.unwrap_or(env!("CARGO_PKG_VERSION"))
        );
        assert!(!super::VERSION.starts_with('v'));
    }
}
