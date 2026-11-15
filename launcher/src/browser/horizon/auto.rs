//! Automatic updates of what the browser installed from Horizon's registry (Settings: "Update
//! registry mods automatically", on by default): at the launcher's start and before Play, each
//! installed registry project moves to its newest version for this game ([`Index::installable`]),
//! checked as an install is. What it never touches:
//!
//! - a game folder inside a modpack's profile: the pack pins its mods by SHA-256 and updates as a
//!   whole, when the player asks;
//! - code mods, which update only when the player asks;
//! - a data pack: it lives in a world, not in the browser's list;
//! - a version for another Minecraft than the one installed, and anything older than what is installed.
//!
//! A content mod (`.hmod`) whose new version drops a block, item, entity, effect, menu or
//! creative tab the installed one has waits for the player (its blocks would vanish from their
//! worlds): the notice says which. The file an update replaces is kept in
//! `mod-browser/replaced/` until the next start: an update cut short (a crash between moving the
//! file and saving the list) is rolled back from it then.
use super::{install_version_guarded, Index};
use crate::browser::{installed, Kind, SourceId, GAME};
use crate::i18n::trf;
use horizon_mod_package::index;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Where replaced files wait for the next start, and their list.
const KEPT_DIR: &str = "mod-browser/replaced";
const KEPT: &str = "mod-browser/replaced.json";

/// What happens to one installed project.
#[derive(Debug, PartialEq)]
pub enum Decision<'a> {
    Update(&'a index::Version),
    /// Not updated automatically, or nothing newer.
    Skip,
}

/// Whether the game folder belongs to a modpack.
pub struct Context {
    pub in_pack: bool,
}

/// What an automatic update did: `Title version` updated, what waits for the player and why,
/// and what failed.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub updated: Vec<String>,
    pub held: Vec<String>,
    pub failed: Vec<String>,
}

impl Report {
    /// The lines the launcher shows: nothing when nothing happened.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.updated.is_empty() {
            out.push(trf(
                "Updated automatically: {list}.",
                &[("list", &self.updated.join(", "))],
            ));
        }
        out.extend(self.held.iter().cloned());
        out.extend(self.failed.iter().cloned());
        out
    }
}

/// Whether game folder `home` is a registry modpack's profile (`pack::home`).
pub fn in_pack(home: &Path) -> bool {
    home.ends_with("home") && home.parent().is_some_and(super::pack::is_pack)
}

/// Whether `entry` updates automatically, and to what.
pub fn decide<'a>(index: &'a Index, entry: &installed::Entry, ctx: &Context) -> Decision<'a> {
    let skip_native = crate::native_status::is_native_file(&entry.file);
    if entry.source != SourceId::Horizon
        || ctx.in_pack
        || skip_native
        || entry.kind == Kind::DataPack
    {
        return Decision::Skip;
    }
    let Some(p) = index.project(&entry.project) else {
        return Decision::Skip;
    };
    let Some(target) = index.installable(p, GAME) else {
        return Decision::Skip;
    };
    // Versions are newest first: only forward, from a version the index still lists.
    let at = |version: &str| p.versions.iter().position(|v| v.version == version);
    let (Some(now), Some(next)) = (at(&entry.version), at(&target.version)) else {
        return Decision::Skip;
    };
    let same_game = p.versions[now].game == target.game;
    let native_next = p.kind == index::Kind::NativeMod;
    if next >= now || !same_game || native_next {
        return Decision::Skip;
    }
    Decision::Update(target)
}

/// The blocks, items, entities, effects, menus and creative tabs a content mod adds.
pub fn content_ids(path: &Path) -> Result<BTreeSet<String>, String> {
    let manifest = horizon_mod_package::Package::read(path)?.manifest;
    let c = horizon_mod_package::Content::read(path, &manifest)?;
    let ids = c.blocks.iter().map(|b| format!("block {}", b.id));
    let ids = ids.chain(c.items.iter().map(|i| format!("item {}", i.id)));
    let ids = ids.chain(c.entity_types.iter().map(|e| format!("entity {}", e.id)));
    let ids = ids.chain(c.effects.iter().map(|e| format!("effect {}", e.id)));
    let ids = ids.chain(c.menu_types.iter().map(|m| format!("menu {}", m.id)));
    Ok(ids
        .chain(
            c.creative_tabs
                .iter()
                .map(|t| format!("creative tab {}", t.id)),
        )
        .collect())
}

/// What the content mod at `old` has that the one at `new` lacks.
pub fn dropped(old: &Path, new: &Path) -> Result<Vec<String>, String> {
    let new = content_ids(new)?;
    Ok(content_ids(old)?.difference(&new).cloned().collect())
}

/// A replaced file kept until the next start, with the entry it had.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Kept {
    entry: installed::Entry,
    /// Its copy, relative to the game folder.
    copy: String,
    /// The launcher process that replaced it: its own updates aren't cleared before it ends.
    pid: u32,
}

fn kept_list(home: &Path) -> Vec<Kept> {
    let list: Vec<Kept> = fs::read_to_string(home.join(KEPT))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    list.into_iter()
        .filter(|kept| {
            Path::new(&kept.copy)
                .strip_prefix(KEPT_DIR)
                .ok()
                .and_then(Path::to_str)
                .and_then(crate::browser::safe_name)
                .is_some()
        })
        .collect()
}

fn save_kept(home: &Path, list: &[Kept]) -> Result<(), String> {
    let path = home.join(KEPT);
    if list.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let part = home.join(format!(
        "mod-browser/.replaced.{}-{id}.part",
        std::process::id()
    ));
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let res = fs::write(&part, text).and_then(|()| fs::rename(&part, &path));
    if let Err(e) = res {
        let _ = fs::remove_file(&part);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

/// Whether the browser's list has `kept`'s project at a newer version whose file is in place.
fn finished(home: &Path, kept: &Kept) -> bool {
    let list = installed::Installed::load(home);
    let Some(now) = list.find(SourceId::Horizon, &kept.entry.project) else {
        return false;
    };
    let hash = now.checksum.strip_prefix("SHA-256:").unwrap_or_default();
    now.version != kept.entry.version
        && now
            .path(home)
            .and_then(|p| index::sha256_file(&p).ok())
            .is_some_and(|h| h.eq_ignore_ascii_case(hash))
}

/// Puts `kept` back: its file where it was and its entry in the browser's list.
fn restore(home: &Path, kept: &Kept) -> Result<(), String> {
    let (Some(target), copy) = (kept.entry.path(home), home.join(&kept.copy)) else {
        return Err(format!("{} is outside the game folder", kept.entry.file));
    };
    fs::copy(&copy, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    let mut list = installed::Installed::load(home);
    let ours =
        |e: &installed::Entry| e.source == SourceId::Horizon && e.project == kept.entry.project;
    if let Some(now) = list.entries.iter().find(|e| ours(e)) {
        if now.file != kept.entry.file {
            if let Some(path) = now.path(home) {
                let _ = fs::remove_file(path);
            }
        }
    }
    list.entries.retain(|e| !ours(e));
    list.entries.push(kept.entry.clone());
    list.save(home)
}

/// At start: each file an earlier launcher replaced is dropped when its update finished, put back
/// when it didn't.
pub fn recover(home: &Path) {
    let _lock = match crate::browser::lock_home(home) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("Cannot lock mod browser in {}: {e}", home.display());
            return;
        }
    };
    let me = std::process::id();
    let (mut mine, earlier): (Vec<Kept>, Vec<Kept>) =
        kept_list(home).into_iter().partition(|k| k.pid == me);
    for k in earlier {
        if !finished(home, &k) {
            if let Err(e) = restore(home, &k) {
                eprintln!("Couldn't put {} back: {e}", k.entry.title);
                mine.push(k);
                continue;
            }
        }
        let _ = fs::remove_file(home.join(&k.copy));
    }
    let _ = save_kept(home, &mine);
}

/// Keeps the file `entry` installed before it is replaced.
fn keep(home: &Path, entry: &installed::Entry) -> Result<Kept, String> {
    let from = entry
        .path(home)
        .ok_or_else(|| format!("{} is outside the game folder", entry.file))?;
    if !index::valid_id(&entry.project) {
        return Err(format!("invalid registry project id {:?}", entry.project));
    }
    let copy = format!("{KEPT_DIR}/{}", entry.project);
    let to = home.join(&copy);
    fs::create_dir_all(home.join(KEPT_DIR)).map_err(|e| e.to_string())?;
    fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
    let kept = Kept {
        entry: entry.clone(),
        copy,
        pid: std::process::id(),
    };
    let mut list = kept_list(home);
    list.retain(|k| k.entry.project != entry.project);
    list.push(kept.clone());
    save_kept(home, &list)?;
    Ok(kept)
}

/// Updates every installed registry project of game folder `home` that [`decide`] says, by the
/// index (fetched at most every ten minutes); None when updates are disabled or it can't be read.
/// An interrupted update is recovered first, even when updates are disabled or offline.
pub fn run(
    home: &Path,
    ctx: &Context,
    enabled: bool,
    progress: &dyn Fn(&str, f32),
) -> Option<Report> {
    // The start's run and Play's never touch a folder at once.
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    recover(home);
    if !enabled {
        return None;
    }
    let index = super::index().ok()?;
    Some(run_with(&index, super::local(), home, ctx, progress))
}

pub(super) fn run_with(
    index: &Index,
    local: bool,
    home: &Path,
    ctx: &Context,
    progress: &dyn Fn(&str, f32),
) -> Report {
    let _lock = match crate::browser::lock_home(home) {
        Ok(guard) => guard,
        Err(e) => {
            return Report {
                failed: vec![e.to_string()],
                ..Report::default()
            };
        }
    };
    recover(home);
    let pending: BTreeSet<String> = kept_list(home)
        .into_iter()
        .filter(|kept| kept.pid != std::process::id())
        .map(|kept| kept.entry.project)
        .collect();
    let mut report = Report::default();
    for entry in installed::Installed::load(home).entries {
        if pending.contains(&entry.project) {
            continue;
        }
        let Decision::Update(v) = decide(index, &entry, ctx) else {
            continue;
        };
        let p = index.project(&entry.project).expect("decide found it");
        match update(index, local, p, v, &entry, home, progress) {
            Ok(None) => {
                report.updated.push(format!("{} {}", p.name, v.version));
            }
            Ok(Some(why)) => report.held.push(why),
            Err(e) => report.failed.push(trf(
                "{title} wasn't updated: {error}",
                &[("title", &p.name), ("error", &e)],
            )),
        }
    }
    report
}

/// Updates `entry` to `v`; Some(why) when it waits for the player instead.
fn update(
    index: &Index,
    local: bool,
    p: &index::Project,
    v: &index::Version,
    entry: &installed::Entry,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<Option<String>, String> {
    let _lock = crate::browser::lock_home(home).map_err(|e| e.to_string())?;
    let list = installed::Installed::load(home);
    let Some(current) = list.find(entry.source, &entry.project) else {
        return Ok(None);
    };
    if current.version != entry.version {
        return Ok(None);
    }
    let old: Option<PathBuf> = entry.path(home).filter(|p| p.is_file());
    let held = RefCell::new(None);
    // Content mods: nothing the worlds may hold goes missing.
    let guard = |new: &Path| -> Result<(), String> {
        let content = entry.file.ends_with(".hmod") && v.file.name.ends_with(".hmod");
        if !content {
            return Ok(());
        }
        let new_package = horizon_mod_package::Package::read(new)?;
        let old_wasm = old
            .as_deref()
            .map(horizon_mod_package::Package::read)
            .transpose()?
            .is_some_and(|package| package.manifest.wasm.is_some());
        if old_wasm || new_package.manifest.wasm.is_some() {
            let why = trf(
                "{title} {version} waits for you: code mods update only when you choose to update them.",
                &[("title", &p.name), ("version", &v.version)],
            );
            *held.borrow_mut() = Some(why.clone());
            return Err(why);
        }
        let Some(old) = &old else {
            return Ok(());
        };
        let gone = dropped(old, new)?;
        if gone.is_empty() {
            return Ok(());
        }
        let why = trf(
            "{title} {version} waits for you: it no longer has {ids}, which your worlds may use. Update it on the Mods page when you're ready.",
            &[
                ("title", &p.name),
                ("version", &v.version),
                ("ids", &gone.join(", ")),
            ],
        );
        *held.borrow_mut() = Some(why.clone());
        Err(why)
    };
    let kept = match &old {
        Some(_) => Some(keep(home, entry)?),
        None => None,
    };
    match install_version_guarded(index, local, p, v, home, &guard, progress) {
        Ok(_) => Ok(None),
        Err(e) => {
            if let Some(kept) = &kept {
                // Leave the backup and journal intact if restoring it fails.
                if !finished(home, kept) {
                    if let Err(restore_error) = restore(home, kept) {
                        let mut list = kept_list(home);
                        if let Some(record) = list
                            .iter_mut()
                            .find(|k| k.entry.project == kept.entry.project)
                        {
                            record.pid = 0;
                        }
                        let _ = save_kept(home, &list);
                        return Err(format!("{e}; restore failed: {restore_error}"));
                    }
                }
                let _ = fs::remove_file(home.join(&kept.copy));
                let mut list = kept_list(home);
                list.retain(|k| k.entry.project != kept.entry.project);
                let _ = save_kept(home, &list);
            }
            match held.into_inner() {
                Some(why) => Ok(Some(why)),
                None => Err(e),
            }
        }
    }
}

#[cfg(test)]
#[path = "auto/tests.rs"]
mod tests;
