//! The commands that read the SDK's data (design §4, design-fit.md 10.2): `check` of a mod
//! folder, `hooks <text>`, `sdk diff <a> <b>`, `fits` and the report `update` prints before it
//! repins. Each reads the pinned build's `surface.json` from `.horizon/sdk/`, or another build's
//! small files through [`crate::sdk_fetch::fetch_build`]; none links the generator's catalog.
use crate::sdk_fetch::{PinnedSdk, ensure_sdk, fetch_build};
use crate::uses_table::{self, Scanned, surface_of};
use horizon_mod_package::{
    Manifest, Package,
    manifest::{MANIFEST, MAX_MANIFEST, MAX_USES},
    sdk::{self, ChangeNote, Index},
};
use mod_surface::child::ChildEntry;
use mod_surface::uses::{Change, Fit, USES_FILE, UsesTable, fits as fit};
use mod_surface::{Surface, leaf};
use std::path::Path;
use std::process::{Command, ExitCode};

/// One searchable record: its id, its doc text and its names in other mod APIs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record<'s> {
    pub id: &'s str,
    pub doc: &'s str,
    pub counterparts: Vec<String>,
}

/// Every record of `surface` a search reads: hooks only, or every kind with its members.
fn records(surface: &Surface, hooks_only: bool) -> Vec<Record<'_>> {
    let mut out: Vec<Record<'_>> = surface
        .hooks
        .iter()
        .map(|hook| {
            let names = &hook.counterparts;
            let counterparts = names
                .neoforge
                .iter()
                .map(|name| format!("NeoForge: {name}"))
                .chain(names.fabric.iter().map(|name| format!("Fabric: {name}")))
                .chain(names.paper.iter().map(|name| format!("Paper: {name}")))
                .collect();
            Record {
                id: &hook.id,
                doc: &hook.doc,
                counterparts,
            }
        })
        .collect();
    if hooks_only {
        return out;
    }
    for item in &surface.traits {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
        for method in &item.methods {
            out.push(Record {
                id: &method.id,
                doc: &method.doc,
                counterparts: vec![],
            });
        }
    }
    for function in surface.handles.iter().chain(&surface.functions) {
        out.push(Record {
            id: &function.id,
            doc: &function.doc,
            counterparts: vec![],
        });
    }
    for item in &surface.macros {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
    }
    for item in &surface.types {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
    }
    for item in &surface.parts {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
    }
    for item in &surface.capabilities {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
    }
    out
}

/// The records every word of `text` occurs in (id, doc or counterparts, any case): those whose
/// id holds every word first, then those a counterpart completes, then the rest, each by id.
pub(crate) fn search<'s>(surface: &'s Surface, text: &str, hooks_only: bool) -> Vec<Record<'s>> {
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return vec![];
    }
    let mut found: Vec<(u8, Record<'s>)> = records(surface, hooks_only)
        .into_iter()
        .filter_map(|record| {
            let id = record.id.to_lowercase();
            let doc = record.doc.to_lowercase();
            let counterparts = record.counterparts.join(" ").to_lowercase();
            let rank = if words.iter().all(|w| id.contains(w)) {
                0
            } else if words
                .iter()
                .all(|w| id.contains(w) || counterparts.contains(w))
            {
                1
            } else if words
                .iter()
                .all(|w| id.contains(w) || counterparts.contains(w) || doc.contains(w))
            {
                2
            } else {
                return None;
            };
            Some((rank, record))
        })
        .collect();
    found.sort_by(|a, b| (a.0, a.1.id).cmp(&(b.0, b.1.id)));
    found.into_iter().map(|(_, record)| record).collect()
}

/// The first line of a doc text.
fn first_line(doc: &str) -> &str {
    doc.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// `hooks <text> [--all]`: the pinned build's hooks (with `--all`, every record) whose id, doc
/// or counterparts hold every word, offline.
pub(crate) fn hooks(args: &[String]) -> Result<(), String> {
    let all = args.iter().any(|a| a == "--all");
    let text = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if text.trim().is_empty() {
        return Err("cargo horizon-mod hooks <text> [--all]".into());
    }
    let sdk = ensure_sdk(Path::new("."))?;
    let surface = surface_of(&sdk)?;
    let found = search(&surface, &text, !all);
    if found.is_empty() {
        let what = if all { "item" } else { "hook" };
        println!("No {what} of {} matches `{text}`.", sdk.name);
        return Ok(());
    }
    for record in &found {
        let names = if record.counterparts.is_empty() {
            String::new()
        } else {
            format!("  ({})", record.counterparts.join(", "))
        };
        println!("{}{names}", record.id);
        let line = first_line(record.doc);
        if !line.is_empty() {
            println!("    {line}");
        }
    }
    Ok(())
}

/// `sdk diff <a> <b>`: what changed in the surface from build `a` to build `b` (Nightly names
/// or shas). Only each build's small files are read, so a pruned build still answers.
pub(crate) fn sdk(args: &[String]) -> Result<(), String> {
    let usage = "cargo horizon-mod sdk diff <build> <build>";
    let [command, a, b] = args else {
        return Err(usage.into());
    };
    if command != "diff" {
        return Err(usage.into());
    }
    let project = Path::new(".");
    let old = fetch_build(project, a)?;
    let new = fetch_build(project, b)?;
    let notes = build_notes(&new)?;
    println!("{} to {}", old.name, new.name);
    print!(
        "{}",
        diff_text(&surface_of(&old)?, &surface_of(&new)?, &new.name, &notes)
    );
    Ok(())
}

/// The diff from `old` to `new`, with the change notes of the build `new` (named `new_name`)
/// under the changes they name: the notes of the changes since the SDK published before it.
fn diff_text(old: &Surface, new: &Surface, new_name: &str, notes: &[ChangeNote]) -> String {
    let mut diff = mod_surface::diff(old, new);
    for note in notes {
        let text = mod_surface::diff::Note {
            file: new_name.to_owned(),
            why: note.why.clone(),
            migrate: note.migrate.clone(),
        };
        // An id the note names may be unchanged here when `old` is not the build `new`
        // was compared with; the note then explains a change this diff does not show.
        diff.attach(&text, &note.ids);
    }
    diff.to_text()
}

/// The mod's display name, from its manifest text.
fn display_name(manifest: &str) -> Result<String, String> {
    Ok(Manifest::parse(manifest)?.name)
}

/// What `fits` judges: the package's or the folder's table, or why the question does not apply.
enum Subject {
    Table {
        name: String,
        table: UsesTable,
    },
    /// The answer to print, with exit status 2.
    NotApplicable(String),
}

/// The table of the `.hmod` at `path`.
fn package_subject(path: &Path) -> Result<Subject, String> {
    let package = Package::read(path)?;
    let manifest = Package::entry(path, MANIFEST, MAX_MANIFEST)?
        .ok_or_else(|| format!("{}: no {MANIFEST}", path.display()))?;
    let manifest = String::from_utf8(manifest).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let name = display_name(&manifest)?;
    let table: toml::Table = toml::from_str(&manifest).map_err(|e| format!("{MANIFEST}: {e}"))?;
    if let Some(why) = mod_source::mod_uses::exact(&table) {
        return Ok(Subject::NotApplicable(exact_text(&name, &why)));
    }
    if package.manifest.native.is_none() && package.manifest.wasm.is_none() {
        return Ok(Subject::NotApplicable(content_text(&name)));
    }
    let Some(bytes) = Package::entry(path, USES_FILE, MAX_USES)? else {
        return Ok(Subject::NotApplicable(format!(
            "{name}'s package was made by an older tool and doesn't say what it uses, so it works on the build it was made for only. Rebuild it with the current `cargo horizon-mod`."
        )));
    };
    let table = UsesTable::read(&bytes).map_err(|e| format!("{USES_FILE}: {e}"))?;
    Ok(Subject::Table { name, table })
}

/// The table of the mod folder at `root`, scanned against its pinned build.
fn folder_subject(entry: ChildEntry, root: &Path) -> Result<Subject, String> {
    let manifest =
        std::fs::read_to_string(root.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let parsed = Manifest::parse(&manifest)?;
    let name = parsed.name;
    if parsed.native.is_none() && parsed.wasm.is_none() {
        return Ok(Subject::NotApplicable(content_text(&name)));
    }
    let table: toml::Table = toml::from_str(&manifest).map_err(|e| format!("{MANIFEST}: {e}"))?;
    if let Some(why) = mod_source::mod_uses::exact(&table) {
        return Ok(Subject::NotApplicable(exact_text(&name, &why)));
    }
    let sdk = ensure_sdk(root)?;
    match uses_table::scan(entry, root, &surface_of(&sdk)?)? {
        Scanned::Uses(uses) => Ok(Subject::Table {
            name,
            table: uses.table(sdk.build),
        }),
        Scanned::Exact(why) => Ok(Subject::NotApplicable(exact_text(&name, &why))),
        Scanned::Unread(why) => Err(why),
    }
}

fn content_text(name: &str) -> String {
    format!("{name} is a content mod, with no code to check against a build; `fits` doesn't apply.")
}

fn exact_text(name: &str, why: &str) -> String {
    format!("{name} is made for exactly one build ({why}), so `fits` doesn't apply.")
}

/// `fits [<file.hmod>] [--build <name>]…`: one line per build (the current Nightly unless
/// `--build` names others) saying whether the mod fits it. Exits 0 when every build fits, 1 when
/// one does not, and 2 when the question cannot be answered or does not apply.
pub(crate) fn fits(entry: ChildEntry, args: &[String]) -> ExitCode {
    match fits_lines(entry, args) {
        Ok(Answer::Fits) => ExitCode::SUCCESS,
        Ok(Answer::DoesNotFit) => ExitCode::from(1),
        // An answer, not a failure: printed plainly.
        Ok(Answer::NotApplicable(text)) => {
            println!("{text}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

/// What `fits` answered, once its lines are printed.
enum Answer {
    Fits,
    DoesNotFit,
    /// The question does not apply: why, not yet printed.
    NotApplicable(String),
}

fn fits_lines(entry: ChildEntry, args: &[String]) -> Result<Answer, String> {
    let package = args.first().filter(|a| !a.starts_with("--")).map(Path::new);
    let mut builds: Vec<&str> = args
        .windows(2)
        .filter(|pair| pair[0] == "--build")
        .map(|pair| pair[1].as_str())
        .collect();
    if builds.is_empty() {
        builds.push("nightly");
    }
    let project = Path::new(".");
    let subject = match package {
        Some(path) => package_subject(path)?,
        None => folder_subject(entry, project)?,
    };
    let (name, table) = match subject {
        Subject::Table { name, table } => (name, table),
        Subject::NotApplicable(text) => return Ok(Answer::NotApplicable(text)),
    };
    println!("{name}:");
    let mut all = true;
    for build in builds {
        let target = fetch_build(project, build)?;
        if target.build == table.made_for {
            println!("{}  made for", target.name);
            continue;
        }
        match fit(&table.entries, &surface_of(&target)?) {
            Fit::Fits => println!("{}  fits", target.name),
            Fit::DoesNotFit(misfits) => {
                all = false;
                let count = misfits
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                let items = if count == 1 { "item" } else { "items" };
                println!("{}  does not fit: {count} {items} changed", target.name);
                for misfit in misfits {
                    println!("    {misfit}");
                }
            }
        }
    }
    Ok(if all {
        Answer::Fits
    } else {
        Answer::DoesNotFit
    })
}

/// What the mod nearest to a removed item may use now, in `new`: the hook with one of the same
/// counterparts first, then the first record whose id holds every word of the item's name.
pub(crate) fn closest<'s>(old: &Surface, new: &'s Surface, id: &str) -> Option<Record<'s>> {
    let old_names: Vec<String> = records(old, true)
        .into_iter()
        .find(|record| record.id == id)
        .map(|record| record.counterparts)
        .unwrap_or_default();
    if !old_names.is_empty()
        && let Some(record) = records(new, true)
            .into_iter()
            .find(|record| record.counterparts.iter().any(|n| old_names.contains(n)))
    {
        return Some(record);
    }
    let words = leaf(id)
        .rsplit("::")
        .next()
        .unwrap_or(id)
        .split(['_', '.'])
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    search(new, &words, false)
        .into_iter()
        .find(|record| record.id != id)
}

/// What moving the pin from the build the mod is pinned to to `to` (the current Nightly when
/// `None`) breaks in this mod, item by item with the lines that use it; printed before `update`
/// repins. A mod made for exactly one build is only named.
pub(crate) fn update_report(
    entry: ChildEntry,
    root: &Path,
    to: Option<&str>,
) -> Result<(), String> {
    let manifest =
        std::fs::read_to_string(root.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let name = display_name(&manifest)?;
    let exact_line = || {
        println!("{name} is made for exactly one build; its declarations are checked as it moves.")
    };
    // Known from the manifest: such a mod has no table to compare, so nothing is fetched for it.
    let table: toml::Table = toml::from_str(&manifest).map_err(|e| format!("{MANIFEST}: {e}"))?;
    if mod_source::mod_uses::exact(&table).is_some() {
        exact_line();
        return Ok(());
    }
    let old = ensure_sdk(root)?;
    let old_surface = surface_of(&old)?;
    let uses = match uses_table::scan(entry, root, &old_surface)? {
        Scanned::Uses(uses) => uses,
        Scanned::Exact(_) => {
            exact_line();
            return Ok(());
        }
        Scanned::Unread(why) => return Err(why),
    };
    let new = fetch_build(root, to.unwrap_or("nightly"))?;
    let new_surface = surface_of(&new)?;
    let notes = migrate_notes(&build_notes(&new)?);
    print!(
        "{}",
        report(&name, &old, &new, &uses, &old_surface, &new_surface, &notes)
    );
    Ok(())
}

/// The change notes `sdk`'s `sdk.json` carries (`surface.notes`): those of the changes since the
/// SDK published before it. The fetch verified the file.
fn build_notes(sdk: &PinnedSdk) -> Result<Vec<ChangeNote>, String> {
    let path = sdk.dir.join(sdk::INDEX);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let index = Index::read(&bytes, &sdk.build).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(index.surface.notes)
}

/// Each changed id's `migrate` line from `notes`; a note without one adds nothing.
fn migrate_notes(notes: &[ChangeNote]) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for note in notes {
        let Some(migrate) = &note.migrate else {
            continue;
        };
        for id in &note.ids {
            out.insert(id.clone(), migrate.clone());
        }
    }
    out
}

/// The text of [`update_report`].
fn report(
    name: &str,
    old: &PinnedSdk,
    new: &PinnedSdk,
    uses: &mod_surface::uses::Uses,
    old_surface: &Surface,
    new_surface: &Surface,
    notes: &std::collections::BTreeMap<String, String>,
) -> String {
    let total = uses.items.len();
    let misfits = match fit(uses.entries(), new_surface) {
        Fit::Fits => vec![],
        Fit::DoesNotFit(misfits) => misfits,
    };
    let mut changed: Vec<&str> = misfits.iter().map(|m| m.id.as_str()).collect();
    changed.dedup();
    let mut text = if changed.is_empty() {
        format!(
            "{name}, from {} to {}: none of the {total} items it uses changed.\n",
            old.name, new.name
        )
    } else {
        format!(
            "{name}, from {} to {}: {} of the {total} items it uses changed.\n\n",
            old.name,
            new.name,
            changed.len()
        )
    };
    for misfit in &misfits {
        let id = &misfit.id;
        text.push_str(&format!("  {misfit}\n"));
        if let Some(used) = uses.items.iter().find(|used| &used.entry.id == id) {
            text.push_str(&format!("      {}\n", uses_table::place(&used.origin)));
        }
        if misfit.change == Change::Removed
            && let Some(record) = closest(old_surface, new_surface, id)
        {
            let names = if record.counterparts.is_empty() {
                String::new()
            } else {
                format!(" ({})", record.counterparts.join(", "))
            };
            text.push_str(&format!("      closest now: {}{names}\n", record.id));
        }
        if let Some(migrate) = notes.get(id) {
            text.push_str(&format!("      note from the change: \"{migrate}\"\n"));
        }
    }
    text
}

/// `check [--uses]` in a mod folder: `cargo check` for the host and for wasm32, the package
/// rules, then what the mod uses of its pinned build (with `--uses`, each item and the line that
/// brought it in).
pub(crate) fn check_mod(entry: ChildEntry, args: &[String]) -> Result<(), String> {
    let root = Path::new(".");
    let package = Package::read(root)?;
    package.directory_files(root)?;
    let code = package.manifest.native.is_some() || package.manifest.wasm.is_some();
    if !code {
        println!("{}: the package rules hold.", package.manifest.name);
        return Ok(());
    }
    // The mod's `horizon-mod-api` is a path into `.horizon/sdk/`: fetch it before Cargo looks.
    let sdk = ensure_sdk(root)?;
    let surface = surface_of(&sdk)?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    for target in [None, Some("wasm32-unknown-unknown")] {
        let mut command = Command::new(&cargo);
        command.arg("check").current_dir(root);
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let status = command.status().map_err(|e| format!("cargo check: {e}"))?;
        if !status.success() {
            let on = target.unwrap_or("this computer");
            return Err(format!("cargo check for {on} failed"));
        }
    }
    match uses_table::scan(entry, root, &surface)? {
        Scanned::Uses(uses) => {
            print!(
                "{}",
                uses_table::summary(&package.manifest.name, &uses, &sdk)
            );
            if args.iter().any(|a| a == "--uses") {
                print!("{}", uses_table::list(&uses));
            }
        }
        Scanned::Exact(_) => println!(
            "{} is made for exactly one build ({}); it has no uses table.",
            package.manifest.name, sdk.name
        ),
        Scanned::Unread(why) => return Err(why),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_surface::{Counterparts, Hook, HookContract, HookEngine, HookKind, Side};

    fn hook(id: &str, doc: &str, neoforge: &[&str]) -> Hook {
        Hook::new(
            id,
            doc,
            HookEngine::default(),
            Counterparts {
                neoforge: neoforge.iter().map(|n| n.to_string()).collect(),
                ..Default::default()
            },
            HookContract {
                version: 1,
                side: Side::Server,
                contract: HookKind::Listen,
                scope: None,
                cancellable: None,
                context: None,
                host: None,
                owner: None,
                answered_by_mod: false,
                arguments: vec![],
                output: None,
                refs: vec![],
            },
        )
    }

    fn surface(hooks: Vec<Hook>) -> Surface {
        Surface::new(
            hooks,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        )
    }

    #[test]
    fn a_search_ranks_id_then_counterpart_then_doc_matches() {
        let surface = surface(vec![
            hook(
                "hook:living_damage_post",
                "After a hit lands.",
                &["LivingDamageEvent.Post"],
            ),
            hook("hook:player_hurt", "When damage reaches a player.", &[]),
            hook(
                "hook:block_broken",
                "A block breaks.",
                &["BlockEvent.BreakEvent"],
            ),
        ]);
        let ids = |text: &str| -> Vec<&str> {
            search(&surface, text, true)
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        assert_eq!(
            ids("damage"),
            ["hook:living_damage_post", "hook:player_hurt"]
        );
        assert_eq!(ids("breakevent"), ["hook:block_broken"]);
        assert_eq!(ids("DAMAGE post"), ["hook:living_damage_post"]);
        assert!(ids("teleport").is_empty());
    }

    fn note(ids: &[&str], why: &str, migrate: Option<&str>) -> ChangeNote {
        ChangeNote {
            ids: ids.iter().map(|id| id.to_string()).collect(),
            why: why.into(),
            migrate: migrate.map(str::to_owned),
        }
    }

    #[test]
    fn the_update_report_takes_each_ids_migrate_line() {
        let notes = migrate_notes(&[
            note(
                &["method:ServerHost::give_item"],
                "Parts",
                Some("items go through Parts under a transaction"),
            ),
            note(&["hook:x"], "only why", None),
        ]);
        assert_eq!(
            notes
                .get("method:ServerHost::give_item")
                .map(String::as_str),
            Some("items go through Parts under a transaction")
        );
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn sdk_diff_prints_the_newer_builds_notes_under_the_changes_they_name() {
        let old = surface(vec![hook("hook:gone", "", &[])]);
        let new = surface(vec![]);
        let text = diff_text(
            &old,
            &new,
            "42c5f70",
            &[note(
                &["hook:gone"],
                "It fired twice.",
                Some("Listen to hook:once."),
            )],
        );
        assert!(text.contains("0 breaking without a note"), "{text}");
        assert!(
            text.contains("    why (42c5f70): It fired twice.\n"),
            "{text}"
        );
        assert!(text.contains("Listen to hook:once."), "{text}");
    }

    #[test]
    fn the_closest_record_shares_a_counterpart_first() {
        let old = surface(vec![hook(
            "hook:block_broken",
            "",
            &["BlockEvent.BreakEvent"],
        )]);
        let new = surface(vec![
            hook("hook:block_broken_v", "", &[]),
            hook("hook:block_break", "", &["BlockEvent.BreakEvent"]),
        ]);
        assert_eq!(
            closest(&old, &new, "hook:block_broken").map(|r| r.id),
            Some("hook:block_break")
        );
    }
}
