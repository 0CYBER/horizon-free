//! Whether an instance plays its native mods (docs/modding/src/how-to/native-mods.md): the mods turned on in its
//! folder against the ones its version was built with (`native-mods.json`, written by
//! `native_mods::build`). The Mods page shows the answer beside its Build button.
use crate::{backend, browser::java, native_mods};
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// A native mod: its id, its version and a digest of its files (None for a build from before
/// `native-mods.json` kept them).
pub type Mod = (String, String, Option<String>);

/// Whether an installed file (`installed::Entry::file`) is a native mod.
pub fn is_native_file(file: &str) -> bool {
    file.strip_prefix(native_mods::INSTALLED)
        .is_some_and(|rest| rest.starts_with('/') && rest.ends_with(".hmod"))
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
/// files at the same version (a reinstalled local `.hmod`), needs a build.
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

/// A `.hmod`'s digest as a build records it: its files without the uses table, which the
/// mod tool packs beside the source but the build's own digest of the copied source never holds
/// (else every mod packed with a uses table read Needs rebuild after each build).
fn archive_digest<'a>(files: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    use horizon_platform::mod_package::mod_surface::uses::USES_FILE;
    native_mods::digest(files.filter(|(name, _)| *name != USES_FILE))
}

/// The mods turned on that the played build doesn't have as they are now: each one's own "Needs
/// build" on the Mods tab, so changing one mod never marks the others. Every one when the
/// version played is no native build.
pub fn unbuilt(enabled: &[Mod], built: Option<&[Mod]>) -> Vec<String> {
    let same = |a: &Mod, b: &Mod| {
        a.0 == b.0 && a.1 == b.1 && (a.2.is_none() || b.2.is_none() || a.2 == b.2)
    };
    enabled
        .iter()
        .filter(|m| !built.is_some_and(|built| built.iter().any(|b| same(m, b))))
        .map(|m| m.0.clone())
        .collect()
}

/// What a native build was made from, as the Mods page and the instance list show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Build {
    /// The release or Nightly it was built on (`native_mods::About::base`).
    pub base: String,
    /// The Horizon commit it compiled.
    pub horizon: String,
    /// The mods whose pin made it compile `horizon` rather than `base`'s own commit.
    pub pinned_by: Vec<String>,
    /// It compiled another commit than its Nightly's, with no record of why: built before the
    /// launcher kept which mods pinned it ([`pinning`] reads them from the instance's mods).
    pub pinned_unknown: bool,
    /// The mods turned on that it was built without, and why.
    pub left_out: Vec<native_mods::LeftOut>,
}

impl Build {
    /// Installed native build `version`'s, from its `native-mods.json`; None for another version.
    pub fn read(version: &str) -> Option<Build> {
        Some(Build::of(native_mods::about(version)?))
    }

    /// What build record `about` says.
    fn of(about: native_mods::About) -> Build {
        // The fork's manifest pins the exact Horizon commit, so a build is never on a commit
        // its mods did not name: nothing is unknown and no pin list is recorded.
        Build {
            pinned_unknown: false,
            base: about.base,
            horizon: about.horizon,
            pinned_by: Vec::new(),
            left_out: about.left_out,
        }
    }

    /// The Horizon it was built on, as a player reads it (`horizon_label`).
    pub fn horizon(&self) -> String {
        horizon_label(&self.base)
    }

    /// The same, short enough for the instance list: `Nightly 10-28`.
    pub fn horizon_short(&self) -> String {
        match crate::nightly::date_of(&self.base) {
            Some(date) => format!("Nightly {}", &date[5..]),
            None => crate::nightly::short_commit(&self.base),
        }
    }
}

/// A release or Nightly as a player reads it beside "Horizon": `v0.2.93` as it is,
/// `nightly-2026-10-29-4baa571` as `Nightly 2026-10-29`.
pub fn horizon_label(version: &str) -> String {
    match crate::nightly::date_of(version) {
        Some(date) => format!("Nightly {date}"),
        None => crate::nightly::short_commit(version),
    }
}

/// Enabled wasm packages that need the patchable engine, never native compilation.
pub fn wasm_points(home: &Path) -> Vec<horizon_platform::mod_package::index::native::Bundle> {
    use horizon_platform::mod_package::manifest::{Package, WASM};
    use std::io::{BufReader, Read};
    wasm_hmods(home)
        .into_iter()
        .filter_map(|path| {
            let file = fs::File::open(&path).ok()?;
            let mut zip = zip::ZipArchive::new(BufReader::new(file)).ok()?;
            // Home also asks for this status. Packages without a wasm module cannot
            // contribute listeners, so avoid opening and validating every resource entry.
            zip.index_for_name(WASM)?;
            let package = Package::read_zip(&mut zip).ok()?;
            if package.manifest.wasm.is_none()
                || package.manifest.inject.is_empty()
                || !java::is_enabled(home, &package.manifest.id)
            {
                return None;
            }
            let mut files = std::collections::BTreeMap::new();
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).ok()?;
                if entry.is_dir() {
                    continue;
                }
                let name = entry.name().ok()?.into_owned();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).ok()?;
                files.insert(name, bytes);
            }
            Some(horizon_platform::mod_package::index::native::Bundle {
                manifest: package.manifest,
                files,
            })
        })
        .collect()
}

/// The native mods turned on in game folder `home`, read from their `.hmod`s. One that can't be
/// read always counts, with its file name and no version, so it needs a build (which says why):
/// the on/off list is keyed by the manifest id, which an unreadable file can't give.
pub fn enabled(home: &Path) -> Vec<Mod> {
    let mut out = Vec::new();
    for path in hmods(home) {
        match horizon_platform::mod_package::index::native::Bundle::read(&path) {
            Ok(b) => {
                if java::is_enabled(home, &b.manifest.id) {
                    let digest =
                        archive_digest(b.files.iter().map(|(n, d)| (n.as_str(), d.as_slice())));
                    out.push((b.manifest.id, b.manifest.version, Some(digest)));
                }
            }
            Err(_) => out.push((stem(&path), String::new(), None)),
        }
    }
    out.extend(wasm_points(home).into_iter().map(|b| {
        let digest = native_mods::digest(b.files.iter().map(|(n, d)| (n.as_str(), d.as_slice())));
        (b.manifest.id, b.manifest.version, Some(digest))
    }));
    out
}

/// The native mods turned on in game folder `home` that pin Horizon commit `revision`.
pub fn pinning(home: &Path, revision: &str) -> Vec<String> {
    bundles_on(home)
        .into_iter()
        .filter(|b| b.manifest.native.as_ref().expect("native bundle").horizon == revision)
        .map(|b| b.manifest.id)
        .collect()
}

/// The readable native mods turned on in game folder `home`.
pub fn bundles_on(home: &Path) -> Vec<horizon_platform::mod_package::index::native::Bundle> {
    hmods(home)
        .into_iter()
        .filter_map(|path| horizon_platform::mod_package::index::native::Bundle::read(&path).ok())
        .filter(|b| java::is_enabled(home, &b.manifest.id))
        .collect()
}

/// Paths declared by a code package, in manifest order.
pub fn declared_internals(manifest: &horizon_platform::mod_package::Manifest) -> Vec<String> {
    manifest
        .internals
        .iter()
        .map(|entry| entry.path.clone())
        .collect()
}

/// A native file not tracked by the registry/browser, including GitHub installs.
#[derive(Clone)]
pub struct Installed {
    pub file: String,
    pub id: String,
    pub version: String,
    pub internals: Vec<String>,
    pub origin: Option<crate::github_mods::Origin>,
    /// The validated package icon, or the package error (the file remains removable).
    pub icon: Result<Option<Arc<horizon_platform::mod_package::icon::Icon>>, String>,
}

impl Installed {
    /// A `.hnmod` launchers before 0.2.70 installed (#17934). It can't load: its row offers only
    /// Remove, since its on/off key is its file name and could switch an installed mod of that id.
    pub fn retired(&self) -> bool {
        self.file.ends_with(".hnmod")
    }

    /// Why the file can't load, shown instead of where it came from.
    pub fn note(&self) -> Option<&'static str> {
        self.retired().then(|| {
            crate::i18n::tr("Old format (.hnmod): ask its author to republish it as .hmod")
        })
    }
}

/// Whether `files` hold a native mod the build card is for; a retired file is not one.
pub fn asks_for_build<'a>(files: impl IntoIterator<Item = &'a Installed>) -> bool {
    files.into_iter().any(|file| !file.retired())
}

/// Discover the actual files, not just GitHub sidecars. Registry files already have a row.
pub fn untracked(home: &Path) -> Vec<Installed> {
    let tracked = crate::browser::installed::Installed::load(home);
    listed(home)
        .into_iter()
        .filter_map(|path| {
            let file = format!("{}/{}", native_mods::INSTALLED, path.file_name()?.to_str()?);
            if !path.is_file() || tracked.entries.iter().any(|e| e.file == file) {
                return None;
            }
            let package = horizon_platform::mod_package::manifest::Package::read(&path);
            let (id, version) = package
                .as_ref()
                .map(|p| (p.manifest.id.clone(), p.manifest.version.clone()))
                .unwrap_or_else(|_| (stem(&path), String::new()));
            // A GitHub install's Remove deletes `<id>.hmod`: an old `.hnmod` is removed as a file.
            let hmod = path.extension().is_some_and(|e| e == "hmod");
            let origin = (hmod && path.file_stem()?.to_str()? == id)
                .then(|| crate::github_mods::origin(home, &id))
                .flatten();
            let internals = package
                .as_ref()
                .map(|p| declared_internals(&p.manifest))
                .unwrap_or_default();
            Some(Installed {
                file,
                id,
                version,
                internals,
                origin,
                icon: package.map(|p| p.icon.map(Arc::new)),
            })
        })
        .collect()
}

/// The mods installed version `version` was built with, or None when it isn't a native build
/// (or its `native-mods.json` can't be read: the Mods page then offers Build).
pub fn built(version: &str) -> Option<Vec<Mod>> {
    Some(parse_built(&record(version)?))
}

/// Native build `version`'s `native-mods.json`.
fn record(version: &str) -> Option<serde_json::Value> {
    if !native_mods::is_native(version) {
        return None;
    }
    let path = backend::version_dir(version).join("native-mods.json");
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// The profile native build `version` was built with (`native-mods.json`'s `profile`).
pub fn built_profile(version: &str) -> Option<native_mods::Profile> {
    serde_json::from_value(record(version)?.get("profile")?.clone()).ok()
}

/// `state`, once the mode counts: a build made in another mode than `chosen` is not this mode's
/// build, so it needs its own.
pub fn in_mode(
    state: State,
    built: Option<native_mods::Profile>,
    chosen: native_mods::Profile,
) -> State {
    if state == State::UpToDate && built != Some(chosen) {
        State::Outdated
    } else {
        state
    }
}

/// The mods a build tried: both those it holds and those it left out.
fn parse_built(about: &serde_json::Value) -> Vec<Mod> {
    let mods = about["mods"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let left_out = about["left_out"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    mods.iter()
        .chain(left_out)
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
/// list and each `.hmod`'s size and time. The Mods page reads the mods again when it changes.
pub fn fingerprint(home: &Path, version: Option<&str>) -> String {
    let mut out = format!("{}\n{version:?}\n", home.display());
    // The on/off list (`browser::java::is_enabled`).
    out += &fs::read_to_string(home.join("horizon-mods.txt")).unwrap_or_default();
    for path in listed(home)
        .into_iter()
        .chain(wasm_hmods(home))
        .flat_map(|p| [p.clone(), p.with_extension("origin.json")])
        .chain([home.join("mod-browser.json")])
    {
        let meta = fs::metadata(&path).ok();
        let time = meta.as_ref().and_then(|m| m.modified().ok());
        out += &format!("\n{} {:?} {time:?}", path.display(), meta.map(|m| m.len()));
    }
    out
}

fn wasm_hmods(home: &Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(home.join(horizon_platform::mod_package::mod_list::DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "hmod"))
        .collect();
    paths.sort();
    paths
}

fn hmods(home: &Path) -> Vec<std::path::PathBuf> {
    native_files(home, &["hmod"])
}

/// [`hmods`] and the `.hnmod`s launchers before 0.2.70 installed (#17934). Those can't be read
/// or built any more, but the Mods page lists them, with why, so they can be removed.
fn listed(home: &Path) -> Vec<std::path::PathBuf> {
    native_files(home, &["hmod", "hnmod"])
}

fn native_files(home: &Path, extensions: &[&str]) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(home.join(native_mods::INSTALLED))
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| {
        p.extension()
            .is_some_and(|e| extensions.iter().any(|x| e == *x))
    });
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
    #[test]
    fn the_uses_table_is_not_part_of_a_mods_digest() {
        let source = [("Cargo.toml", b"a".as_slice()), ("src/lib.rs", b"b")];
        let packed = [source[0], source[1], ("uses.json", b"{}".as_slice())];
        assert_eq!(
            archive_digest(packed.into_iter()),
            archive_digest(source.into_iter())
        );
        assert_eq!(
            archive_digest(source.into_iter()),
            native_mods::digest(source.into_iter())
        );
    }

    #[test]
    fn only_the_changed_mod_needs_a_build() {
        let m = |id: &str, v: &str, d: Option<&str>| (id.into(), v.into(), d.map(Into::into));
        let built = [m("ores", "1.0", Some("a")), m("maps", "2.0", Some("b"))];
        // `maps` updated, `ores` untouched, `tools` new: only the changed and new ones build.
        let enabled = [
            m("ores", "1.0", Some("a")),
            m("maps", "2.1", Some("c")),
            m("tools", "1.0", None),
        ];
        assert_eq!(unbuilt(&enabled, Some(&built)), ["maps", "tools"]);
        // Same version, other files (a reinstalled local `.hmod`): that one builds again.
        assert_eq!(
            unbuilt(&[m("ores", "1.0", Some("z"))], Some(&built)),
            ["ores"]
        );
        // An old record without digests still counts as built.
        assert!(unbuilt(
            &[m("ores", "1.0", Some("a"))],
            Some(&[m("ores", "1.0", None)])
        )
        .is_empty());
        assert_eq!(unbuilt(&enabled[..1], None), ["ores"]);
    }

    use super::*;

    fn m(id: &str, version: &str) -> Mod {
        (id.into(), version.into(), None)
    }

    fn d(id: &str, version: &str, digest: &str) -> Mod {
        (id.into(), version.into(), Some(digest.into()))
    }

    #[test]
    fn declared_paths_use_the_common_owner_for_both_code_targets() {
        let header = "id='owner'\nname='Owner'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\ninternals=[{path='Game',hash='game'},{path='Game::world',kind='field',hash='world'}]\n";
        for target in [
            format!(
                "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                "a".repeat(40),
                "b".repeat(40)
            ),
            format!("[wasm]\nsha512='{}'\ntoolchain='1.99.1'\n", "0".repeat(128)),
        ] {
            let manifest =
                horizon_platform::mod_package::Manifest::parse(&format!("{header}{target}"))
                    .unwrap();
            assert_eq!(declared_internals(&manifest), ["Game", "Game::world"]);
        }
    }

    #[test]
    fn wasm_package_changes_invalidate_the_build_view_cache() {
        let home = std::env::temp_dir().join(format!("wasm-build-view-{}", std::process::id()));
        fs::create_dir_all(home.join(horizon_platform::mod_package::mod_list::DIR)).unwrap();
        let package = home
            .join(horizon_platform::mod_package::mod_list::DIR)
            .join("example.hmod");
        fs::write(&package, b"a").unwrap();
        let before = fingerprint(&home, None);
        fs::write(&package, b"ab").unwrap();
        // Package changes must re-read the manifest to notice added or removed listeners.
        assert_ne!(
            before,
            fingerprint(&home, None),
            "wasm packages invalidate the selection cache"
        );
        let native = home.join(native_mods::INSTALLED);
        fs::create_dir_all(&native).unwrap();
        fs::write(native.join("example.hmod"), b"a").unwrap();
        assert_ne!(
            before,
            fingerprint(&home, None),
            "a native package is a build input"
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn wasm_listeners_select_patchable_build_without_native_compilation() {
        let home = tempfile::tempdir().unwrap();
        let folder = home
            .path()
            .join(horizon_platform::mod_package::mod_list::DIR);
        fs::create_dir_all(&folder).unwrap();
        // A shared wasm member with the required three exports.
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        let mut section = |id: u8, bytes: &[u8]| {
            module.extend([id, bytes.len() as u8]);
            module.extend(bytes);
        };
        section(0, b"\x08dylink.0\x01\x04\0\0\0\0");
        section(1, &[1, 0x60, 0, 0]);
        section(3, &[1, 0]);
        let mut exports = vec![3];
        for name in ["horizon_layout", "horizon_in", "horizon_hook"] {
            exports.push(name.len() as u8);
            exports.extend(name.as_bytes());
            exports.extend([0, 0]);
        }
        section(7, &exports);
        section(10, &[1, 2, 0, 0x0b]);
        for inject in [false, true] {
            let rows = if inject {
                "[[inject]]\nid='count'\nat=\"horizon_client::game::Game<'_>::tick\"\nkind='head'\nwith='tick'\n"
            } else {
                ""
            };
            let manifest = format!("id='wasm_test'\nname='Wasm test'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n{rows}[wasm]\nsha512='{}'\ntoolchain='1.99.1'\nbuilt_with='selection fixture'\n", horizon_platform::mod_package::manifest::sha512_of(&module));
            let bytes = horizon_platform::mod_package::Package::write_files(
                &manifest,
                &[("mod.wasm", &module)],
            )
            .unwrap();
            fs::write(folder.join("test.hmod"), bytes).unwrap();
            java::set_enabled(home.path(), "wasm_test", true).unwrap();
            assert_eq!(enabled(home.path()).len(), usize::from(inject));
            if inject {
                let staged = native_mods::stage_installed(home.path(), home.path().join("staged"))
                    .unwrap()
                    .unwrap();
                let bundles = native_mods::bundles(&staged).unwrap();
                assert_eq!(bundles.len(), 1);
                assert!(bundles[0].manifest.native.is_none());
            }
            java::set_enabled(home.path(), "wasm_test", false).unwrap();
            assert!(enabled(home.path()).is_empty());
        }
    }

    #[test]
    fn github_and_local_files_are_listed_once_and_drive_build_state() {
        use crate::browser::{installed, Kind, SourceId};
        use std::io::Write;
        let home = std::env::temp_dir().join(format!("native-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(native_mods::INSTALLED)).unwrap();
        let pin = "a".repeat(40);
        for (file, id) in [
            ("registry", "registry"),
            ("github", "github"),
            ("renamed", "local"),
        ] {
            let path = home
                .join(native_mods::INSTALLED)
                .join(format!("{file}.hmod"));
            let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
            let manifest = format!(
                "id = '{id}'\nname = '{id}'\nversion = '1.2'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nhorizon = '{pin}'\nmixins = '{pin}'\n"
            );
            for (name, bytes) in [
                ("horizon-mod.toml", manifest.as_str()),
                (
                    "Cargo.toml",
                    "[package]\nname = 'unrelated'\nversion = '9'\n",
                ),
                ("src/lib.rs", "pub fn hooks() {}\n"),
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        let record = crate::github_mods::Origin {
            id: "github".into(),
            version: "1.2".into(),
            owner: "example".into(),
            repo: "mods".into(),
            tag: None,
            commit: pin,
            asset: None,
            sha256: "ab".repeat(32),
        };
        fs::write(
            home.join(native_mods::INSTALLED).join("github.origin.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        installed::Installed {
            entries: vec![installed::Entry {
                source: SourceId::Horizon,
                project: "registry".into(),
                slug: "registry".into(),
                title: "Registry mod".into(),
                kind: Kind::Mod,
                version: "1.2".into(),
                number: "1.2".into(),
                file: format!("{}/registry.hmod", native_mods::INSTALLED),
                checksum: String::new(),
                page: String::new(),
                dependency: false,
                verdict: None,
                skipped: vec![],
                namespaces: vec![],
            }],
        }
        .save(&home)
        .unwrap();
        let files = untracked(&home);
        let seen: Vec<_> = files
            .iter()
            .map(|m| (m.id.as_str(), m.version.as_str(), m.origin.is_some()))
            .collect();
        assert_eq!(seen, [("github", "1.2", true), ("local", "1.2", false)]);
        assert!(files.iter().all(|m| matches!(&m.icon, Ok(None))));
        for id in ["registry", "github", "local"] {
            java::set_enabled(&home, id, true).unwrap();
        }
        assert_eq!(state(&enabled(&home), None), State::NotBuilt);
        let mut pins = pinning(&home, &"a".repeat(40));
        pins.sort();
        assert_eq!(
            pins,
            ["github", "local", "registry"],
            "fit reads the enabled flat packages, including a renamed local file"
        );
        assert!(pinning(&home, &"b".repeat(40)).is_empty());
        for id in ["registry", "github", "local"] {
            java::set_enabled(&home, id, false).unwrap();
        }
        assert_eq!(state(&enabled(&home), None), State::NoMods);
        assert!(
            bundles_on(&home).is_empty(),
            "disabled packages pin nothing"
        );
        assert_eq!(
            untracked(&home).len(),
            2,
            "disabled mods remain in the list"
        );
        let before = fingerprint(&home, None);
        fs::remove_file(home.join(&files[0].file)).unwrap();
        assert_ne!(before, fingerprint(&home, None));
        assert_eq!(
            untracked(&home).len(),
            1,
            "a stale origin is not an installed mod"
        );
        fs::write(
            home.join(native_mods::INSTALLED).join("broken.hmod"),
            b"broken",
        )
        .unwrap();
        let files = untracked(&home);
        assert_eq!(
            files[0].id, "broken",
            "unreadable files can still be removed"
        );
        assert!(files[0].icon.is_err(), "a refusal is not an absent icon");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_retired_hnmod_is_listed_to_remove_but_never_built() {
        use std::io::Write;
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join(native_mods::INSTALLED);
        fs::create_dir_all(&folder).unwrap();
        let before = fingerprint(home.path(), None);
        // As launchers before 0.2.70 installed them (#17934): `horizon-native.json` and
        // `mixins/<id>/`, which today's package reader refuses.
        for name in ["teem_client-0.1.0.hnmod", "zoom.hnmod"] {
            let mut zip = zip::ZipWriter::new(fs::File::create(folder.join(name)).unwrap());
            zip.start_file(
                "horizon-native.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"{\"schema\": 1}").unwrap();
            zip.finish().unwrap();
        }
        let record = crate::github_mods::Origin {
            id: "zoom".into(),
            version: "1.0".into(),
            owner: "example".into(),
            repo: "zoom".into(),
            tag: None,
            commit: "a".repeat(40),
            asset: None,
            sha256: "ab".repeat(32),
        };
        fs::write(
            folder.join("zoom.origin.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert_ne!(
            before,
            fingerprint(home.path(), None),
            "the Mods page reads the folder again"
        );
        let files = untracked(home.path());
        let seen: Vec<_> = files.iter().map(|m| m.file.as_str()).collect();
        assert_eq!(
            seen,
            [
                "horizon-native-mods/teem_client-0.1.0.hnmod",
                "horizon-native-mods/zoom.hnmod"
            ]
        );
        assert!(
            files.iter().all(|m| m.icon.is_err()),
            "each row says why it doesn't load"
        );
        assert!(
            files[1].origin.is_none(),
            "Remove deletes the file itself, not a GitHub install's .hmod"
        );
        for file in &files {
            java::set_enabled(home.path(), &file.id, true).unwrap();
        }
        assert!(enabled(home.path()).is_empty(), "nothing to build");
        assert!(bundles_on(home.path()).is_empty());
    }

    #[test]
    fn a_retired_hnmod_offers_only_remove_and_asks_for_no_build() {
        use std::io::Write;
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join(native_mods::INSTALLED);
        fs::create_dir_all(&folder).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(folder.join("zoom.hnmod")).unwrap());
        zip.start_file(
            "horizon-native.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"{\"schema\": 1}").unwrap();
        zip.finish().unwrap();
        let files = untracked(home.path());
        assert!(files[0].retired());
        assert_eq!(
            files[0].note(),
            Some("Old format (.hnmod): ask its author to republish it as .hmod"),
            "the row says why, not the package reader's refusal"
        );
        assert!(!asks_for_build(&files), "no build card for it alone");

        // A supported mod with the id the retired file's name gives: only it has a switch, so
        // switching the retired row can't turn `zoom` on or off.
        let pin = "a".repeat(40);
        let mut zip = zip::ZipWriter::new(fs::File::create(folder.join("zoom.hmod")).unwrap());
        let manifest = format!(
            "id = 'zoom'\nname = 'zoom'\nversion = '1.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nhorizon = '{pin}'\nmixins = '{pin}'\n"
        );
        for (name, bytes) in [
            ("horizon-mod.toml", manifest.as_str()),
            (
                "Cargo.toml",
                "[package]\nname = 'zoom'\nversion = '1.0.0'\n",
            ),
            ("src/lib.rs", "pub fn hooks() {}\n"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let files = untracked(home.path());
        let rows: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.file.as_str(),
                    f.id.as_str(),
                    f.retired(),
                    f.note().is_some(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("horizon-native-mods/zoom.hmod", "zoom", false, false),
                ("horizon-native-mods/zoom.hnmod", "zoom", true, true)
            ]
        );
        assert!(asks_for_build(&files));
    }

    #[test]
    fn a_build_made_in_another_mode_needs_a_build() {
        use native_mods::Profile::Optimized;
        assert_eq!(in_mode(State::UpToDate, None, Optimized), State::Outdated);
        assert_eq!(
            in_mode(State::UpToDate, Some(Optimized), Optimized),
            State::UpToDate
        );
        assert_eq!(in_mode(State::NoMods, None, Optimized), State::NoMods);
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
        let about = serde_json::json!({
            "mods": [{"id": "zoom", "version": "2.1"}],
            "left_out": [{"id": "hitbox_plus", "version": "1.3.0", "sha256": "aa", "why": "other-version"}],
        });
        let build = parse_built(&about);
        let on = [m("zoom", "2.1"), d("hitbox_plus", "1.3.0", "aa")];
        assert_eq!(state(&on, Some(build.as_slice())), State::UpToDate);
        let on = [m("zoom", "2.1"), m("hitbox_plus", "1.3.1")];
        assert_eq!(state(&on, Some(build.as_slice())), State::Outdated);
        let on = [m("zoom", "2.1"), d("hitbox_plus", "1.3.0", "bb")];
        assert_eq!(state(&on, Some(build.as_slice())), State::Outdated);
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
            ..Build::default()
        };
        assert_eq!(build.horizon(), "Nightly 2026-10-28");
        assert_eq!(build.horizon_short(), "Nightly 10-28");
        let release = Build {
            base: "v0.2.93".into(),
            ..Build::default()
        };
        assert_eq!(release.horizon_short(), "v0.2.93");
        assert_eq!(Build::read("v0.1.0"), None, "not a native build");
    }

    /// A build from before pins were recorded that compiled another commit than its Nightly's
    #[test]
    fn only_hmods_in_the_native_folder_are_native() {
        assert!(is_native_file("horizon-native-mods/veinminer.hmod"));
        assert!(!is_native_file("horizon-native-mods-old/x.hmod"));
        assert!(!is_native_file("horizon-mods/zoom.hmod"));
        assert!(!is_native_file("resourcepacks/x.zip"));
    }

    /// Old switches and notices never count as native-code consent.
    #[test]
    fn old_settings_with_the_native_mods_switch_still_read() {
        let s: backend::Settings =
            serde_json::from_str(r#"{"native_mods": true, "native_mods_noticed": true}"#).unwrap();
        assert!(s.native_mods_allowed_at.is_none());
        let s: backend::Settings = serde_json::from_str(r#"{"native_mods": false}"#).unwrap();
        assert!(s.native_mods_allowed_at.is_none());
        let saved = serde_json::to_string(&s).unwrap();
        assert!(!saved.contains("native_mods"), "{saved}");
    }
}
