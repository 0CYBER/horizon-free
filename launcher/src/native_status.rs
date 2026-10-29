//! Whether an instance plays its native mods (docs/native-mods.md): the mods turned on in its
//! folder against the ones its version was built with (`native-mods.json`, written by
//! `native_mods::build`). The Mods page shows the answer beside its Build button.
use crate::{backend, browser::java, native_mods};
use std::fs;
use std::path::Path;

/// Shown once, before the first native mod is installed.
pub const NOTICE: &str = "Native mods aren't sandboxed: they run with full access to your computer, like any program. Install only mods whose authors you trust. To play with one, build the game on the Mods page; the first build downloads Rust and takes 20 to 30 minutes.";

/// A native mod: its id and version.
pub type Mod = (String, String);

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
    /// The version it plays doesn't hold the mods turned on (or isn't a native build).
    NeedsBuild,
    /// The version it plays was built with exactly these mods.
    UpToDate,
}

/// The state for the mods turned on, `enabled`, and those of the version played, `built` (None
/// when it is no native build). Order doesn't matter; a mod at another version needs a build.
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
    match built {
        Some(built) if sorted(built) == sorted(enabled) => State::UpToDate,
        _ => State::NeedsBuild,
    }
}

/// The native mods turned on in game folder `home`, read from their `.hnmod`s. One that can't be
/// read counts with its file name and no version, so it always needs a build (which says why).
pub fn enabled(home: &Path) -> Vec<Mod> {
    let mut out = Vec::new();
    for path in hnmods(home) {
        let (id, version) = match horizon_mods::index::native::Bundle::read(&path) {
            Ok(b) => (b.manifest.id, b.manifest.version),
            Err(_) => (stem(&path), String::new()),
        };
        if java::is_enabled(home, &id) {
            out.push((id, version));
        }
    }
    out
}

/// The mods installed version `version` was built with, or None when it isn't a native build.
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
        .filter_map(|m| Some((m["id"].as_str()?.into(), m["version"].as_str()?.into())))
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
        (id.into(), version.into())
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
        assert_eq!(state(&on, None), State::NeedsBuild, "not a native build");
        let reordered = [m("zoom", "2.1"), m("veinminer", "1.0")];
        assert_eq!(state(&on, Some(&reordered)), State::UpToDate);
        assert_eq!(
            state(&on, Some(&[m("veinminer", "1.0")])),
            State::NeedsBuild,
            "a mod turned on since"
        );
        assert_eq!(
            state(&on[..1], Some(&on)),
            State::NeedsBuild,
            "a mod turned off since"
        );
        assert_eq!(
            state(&on, Some(&[m("veinminer", "1.1"), m("zoom", "2.1")])),
            State::NeedsBuild,
            "a mod updated since"
        );
    }

    #[test]
    fn native_mods_json_gives_the_built_mods() {
        let about = serde_json::json!({
            "base": "v0.1.0", "horizon": "abc", "rust": "1.99.1",
            "mods": [{"id": "zoom", "version": "2.1"}, {"id": "x"}],
        });
        assert_eq!(parse_built(&about), [m("zoom", "2.1")]);
        assert!(parse_built(&serde_json::json!({})).is_empty());
        assert_eq!(built("v0.1.0"), None, "not a native build");
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
