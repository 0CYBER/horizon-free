//! `horizon-stage ROOT POINTS_JSON INTERNALS_JSON COMMIT`: expose internals, mark points and give functions their replacement slots in a staged
//! source copy, then write the build data (`points.json`, `internals.json`, `loops.json`) with the
//! header naming COMMIT, the source's full commit sha (arismoko/horizon#589). Never run on a working tree.
//!
//! `horizon-stage surface ROOT OUT [--calls CALLS]`: write the surface export (`surface.json`) of
//! the source tree at ROOT to OUT, and with `--calls` the calls and core a wasm mod is checked by
//! (`mod_surface::boundary`) to CALLS. It only reads the tree, so a working tree is fine.
//!
//! `horizon-stage uses SURFACE MOD_DIR [--json OUT --made-for BUILD]`: list
//! what the mod in MOD_DIR uses of SURFACE, each item with the place that brought it in, then
//! how it builds each struct it builds with a literal; with `--json`, write the table a package
//! carries.
//!
//! `horizon-stage fits USES_JSON TARGET_SURFACE`: whether a uses table fits a build's surface,
//! naming each changed item. Exit 0 fits, 1 does not fit, 2 error.
//!
//! `horizon-stage surface-diff OLD NEW [--calls OLD_CALLS NEW_CALLS [--layouts OLD_LAYOUTS NEW_LAYOUTS]]
//! [--notes DIR] [--base-notes DIR]
//! [--json OUT] [--gate]`: print what changed between two surface exports, breaking changes first.
//! `--calls` adds the changes to the calls and core both builds' wasm mods are checked by;
//! `--layouts` adds the two builds' compiled hashes (`horizon-mod-api`'s `wire_layouts` example)
//! for the changes the source does not show. `--notes` attaches
//! the change notes of DIR (`crates/horizon-mod-vocab/changes`) whose file names are not in
//! `--base-notes` (the base tree's folder; required with `--notes`). Exits 1 when a new note
//! names an id that did not change, or with `--gate` when a breaking change has no note; 2 when
//! an input cannot be read or the options are wrong.
use horizon_mod_api_gen::staging::{self, Mode};
use mod_source::mod_uses;
use mod_surface::Surface;
use mod_surface::uses::{Fit, ManifestFacts, Origin, UsesTable, fits, uses_contained};
use std::path::PathBuf;

fn surface(root: &std::path::Path, out: &std::path::Path, calls: Option<&std::path::Path>) {
    let started = std::time::Instant::now();
    // The generator writes source paths into `#[path]`s it then reads: they must be absolute.
    let root = match std::fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("horizon-stage surface: {}: {error}", root.display());
            std::process::exit(1);
        }
    };
    let export = match horizon_mod_api_gen::surface::export(&root) {
        Ok(export) => export,
        Err(error) => {
            eprintln!("horizon-stage surface: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = std::fs::write(out, export.surface.to_json()) {
        eprintln!("horizon-stage surface: {}: {error}", out.display());
        std::process::exit(1);
    }
    if let Some(calls) = calls {
        let table = export
            .boundary
            .as_ref()
            .expect("an export of a tree has its calls");
        if let Err(error) = std::fs::write(calls, table.to_json()) {
            eprintln!("horizon-stage surface: {}: {error}", calls.display());
            std::process::exit(1);
        }
    }
    for note in &export.notes {
        eprintln!("unclassified: {note}");
    }
    let counts: Vec<String> = export
        .counts()
        .iter()
        .map(|(kind, count)| format!("{kind}={count}"))
        .collect();
    println!(
        "surface {} unclassified={} hash={}",
        counts.join(" "),
        export.notes.len(),
        export.surface.hash
    );
    println!(
        "stage_phase=surface seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
}

/// Prints `message` as an error and exits 2.
fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("horizon-stage: {message}");
    std::process::exit(2)
}

fn read_surface(path: &std::path::Path) -> Surface {
    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    Surface::read(&bytes).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
}

fn uses_command(entry: mod_surface::child::ChildEntry, args: &[PathBuf]) {
    let [surface, dir, options @ ..] = args else {
        fail("usage: horizon-stage uses SURFACE MOD_DIR [--json OUT --made-for BUILD]")
    };
    let (mut json, mut made_for) = (None, None);
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let value = options
            .next()
            .unwrap_or_else(|| fail(format!("{} needs a value", option.display())));
        match option.to_str() {
            Some("--json") => json = Some(value.clone()),
            Some("--made-for") => made_for = value.to_str().map(str::to_owned),
            _ => fail(format!("unknown option {}", option.display())),
        }
    }
    let surface = read_surface(surface);
    let input = mod_uses::read(dir).unwrap_or_else(|e| fail(e));
    // No manifest form names a hook version today (the schema refuses `hook_version`).
    let manifest = ManifestFacts {
        names: input.manifest_names,
        hook_versions: Default::default(),
    };
    let uses = uses_contained(
        entry,
        input.sources.iter().map(|(p, t)| (p.as_str(), t.as_str())),
        &manifest,
        &surface,
    )
    .unwrap_or_else(|e| fail(e));
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for used in &uses.items {
        let place = match &used.origin {
            Origin::Source { file, line } => format!("{file}:{line}"),
            Origin::Manifest => "horizon-mod.toml".to_owned(),
            Origin::Via(id) => format!("via {id}"),
        };
        println!("{}  {}  {place}", used.entry.id, used.entry.hash);
        let kind = used.entry.id.split(':').next().unwrap_or_default();
        *kinds.entry(kind).or_default() += 1;
    }
    let counts: Vec<String> = kinds.iter().map(|(k, n)| format!("{k}={n}")).collect();
    println!("uses {} items: {}", uses.items.len(), counts.join(" "));
    for note in uses.items.iter().filter_map(|used| used.note.as_deref()) {
        println!("{note}");
    }
    if let Some(out) = json {
        let made_for = made_for.unwrap_or_else(|| fail("--json needs --made-for BUILD"));
        std::fs::write(&out, uses.table(made_for).to_json())
            .unwrap_or_else(|e| fail(format!("{}: {e}", out.display())));
    }
}

fn fits_command(args: &[PathBuf]) -> ! {
    let [table, target] = args else {
        fail("usage: horizon-stage fits USES_JSON TARGET_SURFACE")
    };
    // Read at most one byte past the cap, so the reader refuses an oversized table by its size.
    let mut bytes = Vec::new();
    std::fs::File::open(table)
        .and_then(|file| {
            use std::io::Read;
            file.take(mod_surface::uses::MAX_USES_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .unwrap_or_else(|e| fail(format!("{}: {e}", table.display())));
    let table =
        UsesTable::read(&bytes).unwrap_or_else(|e| fail(format!("{}: {e}", table.display())));
    match fits(&table.entries, &read_surface(target)) {
        Fit::Fits => {
            println!(
                "fits ({} items, made for {})",
                table.entries.len(),
                table.made_for
            );
            std::process::exit(0)
        }
        Fit::DoesNotFit(misfits) => {
            println!(
                "does not fit: {} of the {} items it uses changed",
                misfits.len(),
                table.entries.len()
            );
            for misfit in &misfits {
                println!("  {misfit}  [{}]", misfit.id);
            }
            std::process::exit(1)
        }
    }
}

fn surface_diff(args: &[PathBuf]) -> ! {
    use horizon_mod_api_gen::surface::notes;
    let [old, new, options @ ..] = args else {
        fail(
            "usage: horizon-stage surface-diff OLD NEW [--calls OLD_CALLS NEW_CALLS [--layouts OLD_LAYOUTS NEW_LAYOUTS]] [--notes DIR] [--base-notes DIR] [--json OUT] [--gate]",
        )
    };
    let (mut notes_dir, mut base_notes, mut json, mut gate) = (None, None, None, false);
    let (mut calls, mut layouts) = (None, None);
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let slot = match option.to_str() {
            Some("--gate") => {
                gate = true;
                continue;
            }
            Some("--calls") => {
                let (Some(old), Some(new)) = (options.next(), options.next()) else {
                    fail("--calls needs OLD_CALLS and NEW_CALLS")
                };
                calls = Some((old, new));
                continue;
            }
            Some("--layouts") => {
                let (Some(old), Some(new)) = (options.next(), options.next()) else {
                    fail("--layouts needs OLD_LAYOUTS and NEW_LAYOUTS")
                };
                layouts = Some((old, new));
                continue;
            }
            Some("--notes") => &mut notes_dir,
            Some("--base-notes") => &mut base_notes,
            Some("--json") => &mut json,
            _ => fail(format!("unknown option {}", option.display())),
        };
        let value = options
            .next()
            .unwrap_or_else(|| fail(format!("{} needs a value", option.display())));
        *slot = Some(value);
    }
    let (old, new) = (read_surface(old), read_surface(new));
    let mut diff = mod_surface::diff(&old, &new);
    let bytes = |path: &PathBuf| {
        std::fs::read(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
    };
    let compiled = layouts.map(|(old, new)| {
        let read = |path: &PathBuf| {
            mod_surface::boundary::Compiled::read(&bytes(path))
                .unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
        };
        (read(old), read(new))
    });
    if let Some((old_calls, new_calls)) = calls {
        let read = |path: &PathBuf| {
            mod_surface::boundary::Boundary::read(&bytes(path))
                .unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
        };
        mod_surface::boundary::apply(
            &mut diff,
            &old,
            &new,
            &read(old_calls),
            &read(new_calls),
            compiled.as_ref().map(|(old, new)| (old, new)),
        );
    } else if compiled.is_some() {
        fail("--layouts needs --calls")
    }
    let mut stale = Vec::new();
    if let Some(dir) = notes_dir {
        // Without the base's notes every older note would read as new, and stale.
        let base = base_notes.unwrap_or_else(|| fail("--notes needs --base-notes"));
        let found = notes::read(dir, base).unwrap_or_else(|e| fail(e));
        for file in found {
            for id in diff.attach(&file.note, &file.ids) {
                stale.push(format!(
                    "change note {} names {id}, which did not change",
                    file.note.file
                ));
            }
        }
    }
    print!("{}", diff.to_text());
    if let Some(out) = json {
        let text = serde_json::to_string_pretty(&diff).expect("a diff serializes") + "\n";
        std::fs::write(out, text).unwrap_or_else(|e| fail(format!("{}: {e}", out.display())));
    }
    let mut failed = !stale.is_empty();
    for line in stale {
        eprintln!("{line}");
    }
    if gate {
        for change in diff.undeclared() {
            eprintln!("breaking change without a change note: {}", change.id);
            failed = true;
        }
    }
    std::process::exit(i32::from(failed))
}

fn main() {
    // The uses scan parses in a child, this program started again: run its part and exit.
    let entry = mod_surface::child::child_entry();
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    match args.first().and_then(|c| c.to_str()) {
        Some("uses") => return uses_command(entry, &args[1..]),
        Some("fits") => fits_command(&args[1..]),
        Some("surface-diff") => surface_diff(&args[1..]),
        _ => {}
    }
    match &args[..] {
        [command, root, out] if command == "surface" => return surface(root, out, None),
        [command, root, out, flag, calls] if command == "surface" && flag == "--calls" => {
            return surface(root, out, Some(calls));
        }
        _ => {}
    }
    let usage = || {
        eprintln!(
            "usage: horizon-stage ROOT POINTS_JSON INTERNALS_JSON COMMIT [--sync-internals]\n       horizon-stage surface ROOT OUT [--calls CALLS]\n       horizon-stage surface-diff OLD NEW [--calls OLD_CALLS NEW_CALLS] [--notes DIR] [--base-notes DIR] [--json OUT] [--gate]\n       horizon-stage uses SURFACE MOD_DIR [--json OUT --made-for BUILD]\n       horizon-stage fits USES_JSON TARGET_SURFACE"
        );
        std::process::exit(2)
    };
    let [root, list, internals, commit, options @ ..] = &args[..] else {
        usage()
    };
    // Discovery never builds or publishes a game.
    let mode = match options {
        [] => Mode::Build,
        [flag] if flag == "--sync-internals" => Mode::Discovery,
        _ => usage(),
    };
    let Some(commit) = commit.to_str() else {
        usage()
    };
    let build = point_shape::build_data::Build {
        commit: commit.to_owned(),
    };
    match staging::stage(root, list, internals, mode, &build) {
        Ok(staging::Staged {
            replace,
            internals: widened,
        }) => {
            println!(
                "gave {} functions a replacement slot in {} files of {} crates",
                replace.functions, replace.files, replace.crates
            );
            let shapes = widened
                .entries
                .iter()
                .filter(|entry| entry.shape_only)
                .count();
            println!(
                "exposed {} internals and listed {} type shapes in {} changed files: {:?}",
                widened.entries.len() - shapes,
                shapes,
                widened.files,
                widened.changed
            );
        }
        Err(error) => {
            eprintln!("horizon-stage: {error}");
            std::process::exit(1);
        }
    }
}
