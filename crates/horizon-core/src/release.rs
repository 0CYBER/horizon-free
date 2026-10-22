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

#[cfg(test)]
mod tests {
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
