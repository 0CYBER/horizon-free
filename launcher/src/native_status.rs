//! Whether an instance plays its native mods (docs/native-mods.md): the mods turned on in its
//! folder against the ones its version was built with (`native-mods.json`, written by
//! `native_mods::build`). The Mods page shows the answer beside its Build button.
use crate::{backend, browser::java, native_mods};
use std::fs;
use std::path::Path;

/// Shown once, before the first native mod is installed.
pub const NOTICE: &str = "Native mods aren't sandboxed: they run with full access to your computer, like any program. Install only mods whose authors you trust. To play with one, build the game on the Mods page; the first build downloads Rust and takes 20 to 30 minutes.";

/// A native mod: its id, its version and a digest of its files (None for a build from before
/// `native-mods.json` kept them).
pub type Mod = (String, String, Option<String>);

/// Whether an installed file (`installed::Entry::file`) is a native mod.
pub fn is_native_file(file: &str) -> bool {
    file.strip_prefix(native_mods::INSTALLED)
        .is_some_and(|rest| rest.starts_with('/') && rest.ends_with(".hnmod"))
}

/// Where an instance's native mods stand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// No native mod is turned on: nothing to build.
    NoMods,
    /// The version it plays isn't a native build.
    NotBuilt,
    /// The version it plays is a native build, made with other mods than those turned on.
    Outdated,
    /// The version it plays was built with exactly these mods.
    UpToDate,
}

/// The state for the mods turned on, `enabled`, and those of the version played, `built` (None
/// when it is no native build). Order doesn't matter; a mod at another version, or with other
/// files at the same version (a reinstalled local `.hnmod`), needs a build.
pub fn state(enabled: &[Mod], built: Option<&[Mod]>) -> State {
    if enabled.is_empty() {
        return State::NoMods;
    }
    let sorted = |mods: &[Mod]| {
        let mut v = mods.to_vec();
        v.sort();
        v.dedup();
        v
    };
    let same = |a: &Mod, b: &Mod| {
        a.0 == b.0 && a.1 == b.1 && (a.2.is_none() || b.2.is_none() || a.2 == b.2)
    };
    match built {
        None => State::NotBuilt,
        Some(built) => {
            let (built, enabled) = (sorted(built), sorted(enabled));
            if built.len() == enabled.len() && built.iter().zip(&enabled).all(|(a, b)| same(a, b)) {
                State::UpToDate
            } else {
                State::Outdated
            }
        }
    }
}

/// What a native build was made from, as the Mods page and the instance list show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Build {
    /// The release or Nightly it was built on (`native_mods::About::base`).
    pub base: String,
}

impl Build {
    /// Installed native build `version`'s, from its `native-mods.json`; None for another version.
    pub fn read(version: &str) -> Option<Build> {
        Some(Build {
            base: native_mods::about(version)?.base,
        })
    }

    /// The Horizon it was built on, as a player reads it (`horizon_label`).
    pub fn horizon(&self) -> String {
        horizon_label(&self.base)
    }

    /// The same, short enough for the instance list: `Nightly 10-28`.
    pub fn horizon_short(&self) -> String {
        match crate::nightly::date_of(&self.base) {
            Some(date) => format!("Nightly {}", &date[5..]),
            None => self.base.clone(),
        }
    }
}

/// A release or Nightly as a player reads it beside "Horizon": `v0.2.93` as it is,
/// `nightly-2026-10-29-4baa571` as `Nightly 2026-10-29`.
pub fn horizon_label(version: &str) -> String {
    match crate::nightly::date_of(version) {
        Some(date) => format!("Nightly {date}"),
        None => version.to_owned(),
    }
}

/// The native mods turned on in game folder `home`, read from their `.hnmod`s. One that can't be
/// read always counts, with its file name and no version, so it needs a build (which says why):
/// the on/off list is keyed by the manifest id, which an unreadable file can't give.
pub fn enabled(home: &Path) -> Vec<Mod> {
    let mut out = Vec::new();
    for path in hnmods(home) {
        match horizon_mods::index::native::Bundle::read(&path) {
            Ok(b) => {
                if java::is_enabled(home, &b.manifest.id) {
                    let digest = native_mods::digest(
                        b.files.iter().map(|(n, d)| (n.as_str(), d.as_slice())),
                    );
                    out.push((b.manifest.id, b.manifest.version, Some(digest)));
                }
            }
            Err(_) => out.push((stem(&path), String::new(), None)),
        }
    }
    out
}

/// The mods installed version `version` was built with, or None when it isn't a native build
/// (or its `native-mods.json` can't be read: the Mods page then offers Build).
pub fn built(version: &str) -> Option<Vec<Mod>> {
    if !native_mods::is_native(version) {
        return None;
    }
    let path = backend::version_dir(version).join("native-mods.json");
    let about: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    Some(parse_built(&about))
}

fn parse_built(about: &serde_json::Value) -> Vec<Mod> {
    let mods = about["mods"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    mods.iter()
        .filter_map(|m| {
            Some((
                m["id"].as_str()?.into(),
                m["version"].as_str()?.into(),
                m["sha256"].as_str().map(String::from),
            ))
        })
        .collect()
}

/// What [`state`] depends on, cheap to read on every frame: the folder, the version, the on/off
/// list and each `.hnmod`'s size and time. The Mods page reads the mods again when it changes.
pub fn fingerprint(home: &Path, version: Option<&str>) -> String {
    let mut out = format!("{}\n{version:?}\n", home.display());
    // The on/off list (`browser::java::is_enabled`).
    out += &fs::read_to_string(home.join("horizon-mods.txt")).unwrap_or_default();
    for path in hnmods(home) {
        let meta = fs::metadata(&path).ok();
        let time = meta.as_ref().and_then(|m| m.modified().ok());
        out += &format!("\n{} {:?} {time:?}", path.display(), meta.map(|m| m.len()));
    }
    out
}

fn hnmods(home: &Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(home.join(native_mods::INSTALLED))
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "hnmod"));
    paths.sort();
    paths
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, version: &str) -> Mod {
        (id.into(), version.into(), None)
    }

    fn d(id: &str, version: &str, digest: &str) -> Mod {
        (id.into(), version.into(), Some(digest.into()))
    }

    #[test]
    fn a_build_is_needed_until_the_version_holds_the_mods_turned_on() {
        let on = [m("veinminer", "1.0"), m("zoom", "2.1")];
        assert_eq!(state(&[], None), State::NoMods);
        assert_eq!(
            state(&[], Some(&on)),
            State::NoMods,
            "nothing on: nothing to build"
        );
        assert_eq!(state(&on, None), State::NotBuilt, "not a native build");
        let reordered = [m("zoom", "2.1"), m("veinminer", "1.0")];
        assert_eq!(state(&on, Some(&reordered)), State::UpToDate);
        assert_eq!(
            state(&on, Some(&[m("veinminer", "1.0")])),
            State::Outdated,
            "a mod turned on since"
        );
        assert_eq!(
            state(&on[..1], Some(&on)),
            State::Outdated,
            "a mod turned off since"
        );
        assert_eq!(
            state(&on, Some(&[m("veinminer", "1.1"), m("zoom", "2.1")])),
            State::Outdated,
            "a mod updated since"
        );
    }

    #[test]
    fn the_same_version_with_other_files_needs_a_build() {
        let on = [d("zoom", "2.1", "aa")];
        assert_eq!(state(&on, Some(&[d("zoom", "2.1", "aa")])), State::UpToDate);
        assert_eq!(
            state(&on, Some(&[d("zoom", "2.1", "bb")])),
            State::Outdated,
            "reinstalled without a version bump"
        );
        assert_eq!(
            state(&on, Some(&[m("zoom", "2.1")])),
            State::UpToDate,
            "a build from before the digest was kept"
        );
    }

    #[test]
    fn native_mods_json_gives_the_built_mods() {
        let about = serde_json::json!({
            "base": "v0.1.0", "horizon": "abc", "rust": "1.99.1",
            "mods": [{"id": "zoom", "version": "2.1"}, {"id": "x"}, {"id": "y", "version": "1", "sha256": "ab"}],
        });
        assert_eq!(parse_built(&about), [m("zoom", "2.1"), d("y", "1", "ab")]);
        assert!(parse_built(&serde_json::json!({})).is_empty());
        assert_eq!(built("v0.1.0"), None, "not a native build");
    }

    #[test]
    fn builds_name_the_horizon_they_were_built_on() {
        assert_eq!(horizon_label("v0.2.93"), "v0.2.93");
        assert_eq!(
            horizon_label("nightly-2026-10-29-4baa571"),
            "Nightly 2026-10-29"
        );
        assert_eq!(horizon_label("nightly-2026-10-29"), "Nightly 2026-10-29");
        let build = Build {
            base: "nightly-2026-10-28-0c02067".into(),
        };
        assert_eq!(build.horizon(), "Nightly 2026-10-28");
        assert_eq!(build.horizon_short(), "Nightly 10-28");
        let release = Build {
            base: "v0.2.93".into(),
        };
        assert_eq!(release.horizon_short(), "v0.2.93");
        assert_eq!(Build::read("v0.1.0"), None, "not a native build");
    }

    #[test]
    fn only_hnmods_in_the_native_folder_are_native() {
        assert!(is_native_file("horizon-native-mods/veinminer.hnmod"));
        assert!(!is_native_file("horizon-native-mods-old/x.hnmod"));
        assert!(!is_native_file("horizon-mods/zoom.hmod"));
        assert!(!is_native_file("resourcepacks/x.zip"));
    }

    /// Settings files from before native mods were on by default still read; their
    /// `native_mods` key is ignored, and the notice stays read.
    #[test]
    fn old_settings_with_the_native_mods_switch_still_read() {
        let s: backend::Settings =
            serde_json::from_str(r#"{"native_mods": true, "native_mods_noticed": true}"#).unwrap();
        assert!(s.native_mods_noticed);
        let s: backend::Settings = serde_json::from_str(r#"{"native_mods": false}"#).unwrap();
        assert!(!s.native_mods_noticed);
        let saved = serde_json::to_string(&s).unwrap();
        assert!(!saved.contains("native_mods"), "{saved}");
    }
}
