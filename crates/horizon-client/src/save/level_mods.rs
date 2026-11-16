//! The mods a world was saved with: the code mods in `level.dat`'s `fml.LoadingModList`, the
//! list NeoForge keeps, and how it compares with the mods loaded now (`ModMismatchEvent`); and
//! Horizon's own records beside it, with the check before a world opens (arismoko/horizon#3856).
//!
//! `level.dat`'s root `horizon` compound, beside `fml`, records the content mods the world was
//! saved with (`ContentModList`, the same `{ModId, ModVersion}` compounds as
//! `fml.LoadingModList`), the namespaces of the mod block states registered when it was saved
//! (`BlockNamespaces`), and the game commit that last saved it (`LastSavedCommit`). Before a world
//! opens, [`missing`] names each saved mod that is no longer loaded and each saved block namespace
//! that nothing registers now: opening the world anyway turns those blocks into air, with what
//! they hold, for good.
use super::{Level, Tag, compound_or_empty, put, string, text};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
};

/// A code mod as the list names it: one `{ModId, ModVersion}` compound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModRecord {
    pub id: String,
    /// The mod's `horizon-mod.toml` version; `""` when it wasn't known.
    pub version: String,
}

/// The code mods this process loaded, once the game's start said which.
static LOADED: RwLock<Option<Vec<ModRecord>>> = RwLock::new(None);

/// The code mods loaded now, in load order, each with its version (`""` when unknown): what
/// every `level.dat` written from here on records. The game's start says it once. Returns the
/// list it replaced: the list is the whole process's, so a test that sets one puts that back.
pub fn set_loaded(mods: Option<Vec<ModRecord>>) -> Option<Vec<ModRecord>> {
    std::mem::replace(
        &mut *LOADED.write().unwrap_or_else(|e| e.into_inner()),
        mods,
    )
}

/// The loaded mods `set_loaded` gave; `None` before it (a tool or a test that didn't say).
pub fn loaded() -> Option<Vec<ModRecord>> {
    LOADED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The code mods a level.dat's root lists; `None` without the list (vanilla, or saved before
/// Horizon wrote it).
fn saved_code(data: &Tag) -> Option<Vec<ModRecord>> {
    let Some(Tag::List(mods)) = data.get("fml").and_then(|fml| fml.get("LoadingModList")) else {
        return None;
    };
    Some(records(mods))
}

/// The code mods, then the content mods, a level.dat's root lists; `None` without either list.
pub(super) fn saved(data: &Tag) -> Option<Vec<ModRecord>> {
    let content = horizon_records(data).content_mods;
    match (saved_code(data), content) {
        (None, None) => None,
        (code, content) => Some(code.into_iter().chain(content).flatten().collect()),
    }
}

/// The mods a `{ModId, ModVersion}` list names; a missing or malformed field reads as `""`.
pub(super) fn records(list: &[Tag]) -> Vec<ModRecord> {
    list.iter()
        .map(|entry| ModRecord {
            id: text(entry.get("ModId")).unwrap_or_default(),
            version: text(entry.get("ModVersion")).unwrap_or_default(),
        })
        .collect()
}

/// `mods` as a `{ModId, ModVersion}` list.
pub(super) fn list(mods: &[ModRecord]) -> Tag {
    Tag::List(
        mods.iter()
            .map(|m| Tag::compound([("ModId", string(&m.id)), ("ModVersion", string(&m.version))]))
            .collect(),
    )
}

/// Puts the loaded mods' list into `data`'s `fml`, in place of the one there, keeping the rest
/// of `fml` (left as it is while the loaded mods aren't known), and Horizon's records into its
/// `horizon` compound.
pub(super) fn write(data: &mut Tag) {
    if let Some(mods) = loaded() {
        write_list(data, &mods);
    }
    write_horizon(data);
}

/// `write` with the list given.
fn write_list(data: &mut Tag, mods: &[ModRecord]) {
    let mut fml = compound_or_empty(data.get("fml").cloned());
    put(&mut fml, "LoadingModList", list(mods));
    put(data, "fml", fml);
}

/// The saved mods that differ from `loaded`, NeoForge's `ModMismatchEvent`: one saved with another
/// version (including an unknown saved version) or no longer loaded. A mod loaded now that the
/// world wasn't saved with is no mismatch.
pub(super) fn mismatches<'a>(
    saved: impl IntoIterator<Item = &'a ModRecord>,
    loaded: &[ModRecord],
) -> Vec<mod_api::ModMismatch> {
    saved
        .into_iter()
        .filter_map(|saved| {
            if saved.id == "minecraft" {
                return None;
            }
            let current = loaded.iter().find(|m| m.id == saved.id);
            let differs = current.is_none_or(|current| {
                match (
                    horizon_mod_package::dependencies::parse_version(&saved.version),
                    horizon_mod_package::dependencies::parse_version(&current.version),
                ) {
                    (Ok(saved), Ok(current)) => saved != current,
                    _ => saved.version != current.version,
                }
            });
            differs.then(|| mod_api::ModMismatch {
                mod_id: saved.id.clone(),
                saved_version: saved.version.clone(),
                current_version: current.map(|m| m.version.clone()),
            })
        })
        .collect()
}

impl Level {
    /// The saved code and content mods that differ from `loaded` (the code and content mods
    /// loaded now): [`mismatches`]. A world without either list has none.
    pub fn mod_mismatches(&self, loaded: &[ModRecord]) -> Vec<mod_api::ModMismatch> {
        mismatches(self.saved_mods.iter().flatten(), loaded)
    }
}

/// The root compound Horizon keeps its own world records in.
pub const COMPOUND: &str = "horizon";
const CONTENT_MODS: &str = "ContentModList";
const BLOCK_NAMESPACES: &str = "BlockNamespaces";
const BUILD_COMMIT: &str = "LastSavedCommit";

/// The content-only mods this process loaded, once startup said which.
static CONTENT: RwLock<Option<Vec<ModRecord>>> = RwLock::new(None);
/// The namespaces of the mod block states this process registers, once startup said which.
static NAMESPACES: RwLock<Option<Vec<String>>> = RwLock::new(None);

/// The content-only mods loaded now, each with its version (`""` when unknown): what every
/// `level.dat` written from here on records. Returns the list it replaced, for a test to put back.
pub fn set_content_mods(mods: Option<Vec<ModRecord>>) -> Option<Vec<ModRecord>> {
    std::mem::replace(
        &mut *CONTENT.write().unwrap_or_else(|e| e.into_inner()),
        mods,
    )
}

/// The content-only mods `set_content_mods` gave; `None` before it.
pub fn content_mods() -> Option<Vec<ModRecord>> {
    CONTENT.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The block namespaces registered now, from the block names of the live registry; `minecraft`
/// is left out. Returns the list it replaced, for a test to put back.
pub fn set_block_namespaces(namespaces: Option<Vec<String>>) -> Option<Vec<String>> {
    std::mem::replace(
        &mut *NAMESPACES.write().unwrap_or_else(|e| e.into_inner()),
        namespaces,
    )
}

/// The block namespaces `set_block_namespaces` gave; `None` before it.
pub fn block_namespaces() -> Option<Vec<String>> {
    NAMESPACES.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The sorted, distinct namespaces of `names` other than `minecraft`; a name without one is
/// vanilla's.
pub fn mod_namespaces<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut namespaces: Vec<String> = names
        .into_iter()
        .filter_map(|name| name.split_once(':').map(|(namespace, _)| namespace))
        .filter(|namespace| *namespace != "minecraft")
        .map(str::to_owned)
        .collect();
    namespaces.sort();
    namespaces.dedup();
    namespaces
}

/// The commit this build was made from, when the build knows it.
pub fn build_commit() -> Option<&'static str> {
    option_env!("HORIZON_COMMIT").filter(|commit| !commit.is_empty())
}

/// What a `level.dat`'s `horizon` compound says; each `None` when the world has none (vanilla,
/// or saved before Horizon wrote it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HorizonRecords {
    pub content_mods: Option<Vec<ModRecord>>,
    pub block_namespaces: Option<Vec<String>>,
    /// The game commit that last saved the world. A malformed marker reads as none, with a log
    /// line.
    pub build_commit: Option<String>,
}

/// The records in `root`, a `level.dat`'s root.
pub fn horizon_records(root: &Tag) -> HorizonRecords {
    let Some(compound) = root.get(COMPOUND) else {
        return HorizonRecords::default();
    };
    let content_mods = match compound.get(CONTENT_MODS) {
        Some(Tag::List(mods)) => Some(records(mods)),
        _ => None,
    };
    let block_namespaces = match compound.get(BLOCK_NAMESPACES) {
        Some(Tag::List(namespaces)) => Some(
            namespaces
                .iter()
                .filter_map(|namespace| text(Some(namespace)))
                .collect(),
        ),
        _ => None,
    };
    let build_commit = match compound.get(BUILD_COMMIT) {
        None => None,
        Some(tag) => match text(Some(tag)) {
            Some(commit) if !commit.is_empty() => Some(commit),
            _ => {
                log::warn!("Horizon: level.dat's {COMPOUND}.{BUILD_COMMIT} is malformed; ignored");
                None
            }
        },
    };
    HorizonRecords {
        content_mods,
        block_namespaces,
        build_commit,
    }
}

/// Puts what this process loaded into `root`'s `horizon` compound, keeping its other keys: the
/// content mods and block namespaces once startup said them (left as they are before), and this
/// build's commit (removed when the build does not know it, so the marker never names an
/// earlier build than the last one to save).
fn write_horizon(root: &mut Tag) {
    write_with(
        root,
        content_mods().as_deref(),
        block_namespaces().as_deref(),
        build_commit(),
    );
}

/// `write` with what it records given.
fn write_with(
    root: &mut Tag,
    content: Option<&[ModRecord]>,
    namespaces: Option<&[String]>,
    commit: Option<&str>,
) {
    let mut compound = compound_or_empty(root.get(COMPOUND).cloned());
    if let Some(content) = content {
        put(&mut compound, CONTENT_MODS, list(content));
    }
    if let Some(namespaces) = namespaces {
        let list = namespaces.iter().map(|namespace| string(namespace));
        put(&mut compound, BLOCK_NAMESPACES, Tag::List(list.collect()));
    }
    match commit {
        Some(commit) => put(&mut compound, BUILD_COMMIT, string(commit)),
        None => {
            if let Tag::Compound(map) = &mut compound {
                map.remove(&BUILD_COMMIT.into());
            }
        }
    }
    put(root, COMPOUND, compound);
}

/// What is loaded now, to check a world against.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub code: Vec<ModRecord>,
    pub content: Vec<ModRecord>,
    pub namespaces: Vec<String>,
}

impl Loaded {
    /// What this process loaded, as startup said it; empty where it said nothing.
    pub fn now() -> Self {
        Self {
            code: loaded().unwrap_or_default(),
            content: content_mods().unwrap_or_default(),
            namespaces: block_namespaces().unwrap_or_default(),
        }
    }

    /// The code and content mods together, code first.
    pub fn mods(&self) -> Vec<ModRecord> {
        self.code.iter().chain(&self.content).cloned().collect()
    }

    fn has_mod(&self, id: &str) -> bool {
        self.code.iter().chain(&self.content).any(|m| m.id == id)
    }
}

/// A mod the world was saved with whose content opening the world now would lose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingMod {
    /// The mod's id, or the block namespace when no saved mod has that id.
    pub id: String,
    /// Its version when the world was saved; `""` when it wasn't known.
    pub saved_version: String,
    /// Saved as a code mod (`fml.LoadingModList`) and not loaded now.
    pub code: bool,
    /// Saved as a content mod and not loaded now.
    pub content: bool,
    /// Its namespace had block states when the world was saved, and nothing registers one now.
    pub blocks: bool,
}

/// Every saved mod that is no longer loaded and every saved block namespace nothing registers
/// now, by id. A mod whose version changed is no loss here (`mod_mismatch` still hears it); nor is
/// a world saved before these records, for what it does not record.
pub fn missing(root: &Tag, loaded: &Loaded) -> Vec<MissingMod> {
    let saved_code = saved_code(root).unwrap_or_default();
    let saved = horizon_records(root);
    let saved_content = saved.content_mods.unwrap_or_default();
    let mut missing: Vec<MissingMod> = Vec::new();
    // The entry for `id`, added with `version` when there is none yet.
    fn entry<'m>(missing: &'m mut Vec<MissingMod>, id: &str, version: &str) -> &'m mut MissingMod {
        let index = match missing.iter().position(|m| m.id == id) {
            Some(index) => index,
            None => {
                missing.push(MissingMod {
                    id: id.to_owned(),
                    saved_version: version.to_owned(),
                    code: false,
                    content: false,
                    blocks: false,
                });
                missing.len() - 1
            }
        };
        &mut missing[index]
    }
    for record in &saved_code {
        if record.id != "minecraft" && !loaded.has_mod(&record.id) {
            entry(&mut missing, &record.id, &record.version).code = true;
        }
    }
    for record in &saved_content {
        if record.id != "minecraft" && !loaded.has_mod(&record.id) {
            entry(&mut missing, &record.id, &record.version).content = true;
        }
    }
    for namespace in saved.block_namespaces.iter().flatten() {
        if namespace == "minecraft" || loaded.namespaces.contains(namespace) {
            continue;
        }
        let version = saved_code
            .iter()
            .chain(&saved_content)
            .find(|m| &m.id == namespace)
            .map_or("", |m| m.version.as_str());
        entry(&mut missing, namespace, version).blocks = true;
    }
    missing.sort_by(|a, b| a.id.cmp(&b.id));
    missing
}

/// [`missing`] for the world in `dir`, from its `level.dat` (or `level.dat_old` when that does
/// not read). Reads only: nothing in `dir` changes.
pub fn missing_in(dir: &Path, loaded: &Loaded) -> Vec<MissingMod> {
    super::read_level_dat(dir).map_or_else(Vec::new, |root| missing(&root, loaded))
}

/// `mod_mismatch`'s differences for the world in `dir`: its saved code and content mods against
/// the loaded ones (`Level::mod_mismatches`); `None` without a saved list.
pub fn mismatches_in(dir: &Path, loaded: &Loaded) -> Option<Vec<mod_api::ModMismatch>> {
    let saved = saved(&super::read_level_dat(dir)?)?;
    Some(mismatches(&saved, &loaded.mods()))
}

/// The answer `mod_mismatch` gave before a world opened, kept for the open so the hook fires
/// once, and whether the player chose to open that world anyway.
struct Answer {
    folder: PathBuf,
    mismatches: Vec<mod_api::ModMismatch>,
    resolved: Vec<mod_api::ModMismatchResolution>,
    confirmed: bool,
}
static ANSWERED: Mutex<Option<Answer>> = Mutex::new(None);
/// Held by tests that use the kept answer, which is the whole process's.
#[cfg(test)]
pub(crate) static TEST_ANSWER: Mutex<()> = Mutex::new(());

fn answered() -> std::sync::MutexGuard<'static, Option<Answer>> {
    ANSWERED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Keeps `mod_mismatch`'s answer for the world in `dir`, heard before it opened, in place of
/// any other.
pub fn remember_answer(
    dir: &Path,
    mismatches: Vec<mod_api::ModMismatch>,
    resolved: Vec<mod_api::ModMismatchResolution>,
) {
    *answered() = Some(Answer {
        folder: same_path(dir),
        mismatches,
        resolved,
        confirmed: false,
    });
}

/// The player chose to open the world in `dir` with its missing mods: its next open goes ahead
/// without asking again.
pub fn confirm(dir: &Path) {
    if let Some(answer) = answered().as_mut().filter(|a| a.folder == same_path(dir)) {
        answer.confirmed = true;
    }
}

/// Whether the player chose to open the world in `dir` with its missing mods.
pub fn is_confirmed(dir: &Path) -> bool {
    answered()
        .as_ref()
        .is_some_and(|a| a.confirmed && a.folder == same_path(dir))
}

/// The answer kept for the world in `dir`, taken: `None` when the hook was not asked before this
/// open (an answer kept for another world is dropped).
pub fn take_answer(
    dir: &Path,
) -> Option<(
    Vec<mod_api::ModMismatch>,
    Vec<mod_api::ModMismatchResolution>,
)> {
    let answer = answered().take()?;
    (answer.folder == same_path(dir)).then_some((answer.mismatches, answer.resolved))
}

/// Drops the answer kept for a world that did not open after all.
pub fn forget_answer() {
    answered().take();
}

fn same_path(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned())
}

/// The missing mods no `mod_mismatch` listener resolved.
pub fn unresolved(
    missing: Vec<MissingMod>,
    resolved: &[mod_api::ModMismatchResolution],
) -> Vec<MissingMod> {
    missing
        .into_iter()
        .filter(|m| !resolved.iter().any(|r| r.mod_id == m.id))
        .collect()
}

/// One line for `m`: its id, its saved version and what opening the world loses.
pub fn describe(m: &MissingMod) -> String {
    let version = if m.saved_version.is_empty() {
        crate::lang::get("horizon.missing_mods.unknown_version")
    } else {
        m.saved_version.clone()
    };
    let kinds = affected_kinds(m);
    crate::lang::tr("horizon.missing_mods.mod", &[&m.id, &version, &kinds])
}

/// The translated kinds of saved mod state affected by opening without this mod.
pub fn affected_kinds(m: &MissingMod) -> String {
    let kinds: Vec<String> = [
        (m.code, "horizon.missing_mods.kind.code"),
        (m.content, "horizon.missing_mods.kind.content"),
        (m.blocks, "horizon.missing_mods.kind.blocks"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, key)| crate::lang::get(key))
    .collect();
    kinds.join(", ")
}

/// The command-line flag that lets a dedicated server open a world with missing mods.
pub const ALLOW_FLAG: &str = "allowMissingMods";

/// A dedicated server's lines for `missing`: one per mod, then the one naming the switch.
pub fn refusal_lines(missing: &[MissingMod]) -> Vec<String> {
    let mut lines: Vec<String> = missing.iter().map(describe).collect();
    lines.push(crate::lang::tr(
        "horizon.missing_mods.server.refused",
        &[&ALLOW_FLAG],
    ));
    lines
}

#[cfg(test)]
#[path = "level_mods_records_tests.rs"]
mod records_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::{read_dat, write_dat};
    use std::fs;

    fn record(id: &str, version: &str) -> ModRecord {
        ModRecord {
            id: id.into(),
            version: version.into(),
        }
    }

    #[test]
    fn malformed_id_is_reported_as_missing() {
        let root = Tag::compound([(
            "fml",
            Tag::compound([(
                "LoadingModList",
                Tag::List(vec![
                    Tag::compound([("ModId", Tag::Int(7))]),
                    Tag::Compound(Default::default()),
                ]),
            )]),
        )]);
        let level = Level {
            saved_mods: saved(&root),
            ..Level::default()
        };
        let differences = level.mod_mismatches(&[]);
        assert_eq!(differences.len(), 2);
        assert!(
            differences
                .iter()
                .all(|m| m.mod_id.is_empty() && m.current_version.is_none())
        );
    }

    /// The loaded mods replace the list the file had and the other `fml` keys stay; a write
    /// before the loaded mods are known leaves the list alone, and a world without one has none.
    /// The loaded list is given to `write_list`, not set: the process-wide one would reach
    /// the level.dat other tests write meanwhile.
    #[test]
    fn the_level_data_records_the_loaded_mods() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let entry = |id: &str, version: &str| {
            Tag::compound([
                ("ModId", Tag::String(id.into())),
                ("ModVersion", Tag::String(version.into())),
            ])
        };
        write_dat(
            &dir.join("level.dat"),
            Tag::compound([
                (
                    "Data",
                    Tag::compound([("LevelName", Tag::String("Test".into()))]),
                ),
                (
                    "fml",
                    Tag::compound([
                        ("Other", Tag::Int(7)),
                        ("LoadingModList", Tag::List(vec![entry("old", "0.1.0")])),
                    ]),
                ),
            ]),
        )
        .unwrap();
        let mut level = Level::read(&dir).unwrap();
        assert_eq!(level.saved_mods, Some(vec![record("old", "0.1.0")]));
        level.write(&dir).unwrap();
        assert_eq!(
            Level::read(&dir).unwrap().saved_mods,
            Some(vec![record("old", "0.1.0")])
        );
        let mut data = read_dat(&dir.join("level.dat")).unwrap();
        write_list(&mut data, &[record("first", "1.2.0"), record("second", "")]);
        assert_eq!(
            saved(&data),
            Some(vec![record("first", "1.2.0"), record("second", "")])
        );
        let fml = data.get("fml").unwrap();
        assert_eq!(fml.get("Other"), Some(&Tag::Int(7)));
        assert_eq!(
            fml.get("LoadingModList"),
            Some(&Tag::List(vec![
                entry("first", "1.2.0"),
                entry("second", "")
            ]))
        );
        assert_eq!(
            saved(&Tag::compound([(
                "LevelName",
                Tag::String("Vanilla".into())
            )])),
            None
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// `ModMismatchEvent`: a saved mod with another version, or no longer loaded, and nothing
    /// else: not a mod loaded since, including versions unknown on either side, not a world without
    /// the list.
    #[test]
    fn mod_mismatches_name_changed_and_missing_mods_only() {
        let mismatch = |mod_id: &str, saved: &str, current: Option<&str>| mod_api::ModMismatch {
            mod_id: mod_id.into(),
            saved_version: saved.into(),
            current_version: current.map(str::to_owned),
        };
        let loaded = [
            record("same", "1.0.0"),
            record("changed", "2.0.0"),
            record("unknown_now", ""),
            record("known_now", "3.0.0"),
            record("added", "1.0.0"),
            record("overflow", "99999999999999999999999999999999999999999999"),
            record("minecraft", "99"),
            record("downgrade", "3.0.0"),
        ];
        assert!(Level::default().mod_mismatches(&loaded).is_empty());
        let level = Level {
            saved_mods: Some(vec![
                record("same", "1"),
                record("minecraft", "0"),
                record("downgrade", "4.0.0"),
                record("changed", "1.0.0"),
                record("unknown_now", "1.0.0"),
                record("known_now", ""),
                record("gone", "0.5.0"),
                record("overflow", "1"),
            ]),
            ..Level::default()
        };
        assert_eq!(
            level.mod_mismatches(&loaded),
            vec![
                mismatch("downgrade", "4.0.0", Some("3.0.0")),
                mismatch("changed", "1.0.0", Some("2.0.0")),
                mismatch("unknown_now", "1.0.0", Some("")),
                mismatch("known_now", "", Some("3.0.0")),
                mismatch("gone", "0.5.0", None),
                mismatch(
                    "overflow",
                    "1",
                    Some("99999999999999999999999999999999999999999999")
                ),
            ]
        );
        let unchanged = Level {
            saved_mods: Some(loaded.to_vec()),
            ..Level::default()
        };
        assert!(unchanged.mod_mismatches(&loaded).is_empty());
    }
}
