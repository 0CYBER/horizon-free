//! Capture generated hook artifacts for a byte-for-byte refactor comparison.
//! Run with a destination directory argument (empty or new) before and after the split, then `diff -r`.
//! The generator's cache (`incremental-*`, and `interfaces-*` from older builds, keyed by this executable) and its phase timings
//! (`points-timing.txt`) are removed from the destination, so two runs of different builds compare equal.
//! Every error names the step and the path that failed.
use std::{fs, path::Path, path::PathBuf};

/// Runs one step, naming it in its error.
fn step<T>(name: &str, run: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    run().map_err(|error| format!("{name}: {error}"))
}

fn write(out: &Path, name: &str, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    let path = out.join(name);
    fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn json(value: &impl serde::Serialize) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value).map_err(|e| e.to_string())
}

fn main() -> Result<(), String> {
    horizon_mod_api_gen::generated_on_stdout();
    let root = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let out = PathBuf::from(std::env::args().nth(1).expect("destination directory"));
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    // The cleanup below removes `incremental-*` and `interfaces-*` folders: only ever ones this run made, in a destination that was empty.
    let occupied = fs::read_dir(&out)
        .map_err(|e| format!("{}: {e}", out.display()))?
        .next()
        .is_some();
    if occupied {
        return Err(format!(
            "{}: the destination has to be empty or new",
            out.display()
        ));
    }
    step("points.json", || {
        write(
            &out,
            "points.json",
            json(&horizon_mod_api_gen::points::game_points(&root)?)?,
        )
    })?;
    step("generated API", || {
        horizon_mod_api_gen::generate(&root, &out, horizon_mod_api_gen::Target::Api)
    })?;
    step("generated points", || {
        horizon_mod_api_gen::generate_points(&root, &out)
    })?;
    // The published loops and the exposed internals, as `loops.json` and `internals.json` have them.
    step("loops.json", || {
        let loops = horizon_mod_api_gen::loops::collect(&root)?;
        write(
            &out,
            "loops.json",
            json(&serde_json::json!({ "loops": loops }))?,
        )
    })?;
    step("internals.json", || {
        let internals = horizon_mod_api_gen::internals::rule_catalog(&root)?;
        write(
            &out,
            "internals.json",
            json(&serde_json::json!({ "internals": internals }))?,
        )
    })?;
    step("catalog.json", || {
        write(
            &out,
            "catalog.json",
            horizon_mod_api_gen::neoforge_catalog(&root)?,
        )
    })?;
    step("hook-notes.md", || {
        write(
            &out,
            "hook-notes.md",
            horizon_mod_api_gen::generate_hook_notes(&root)?,
        )
    })?;
    // The generator's own scratch: a cache keyed by this executable and phase timings.
    for entry in fs::read_dir(&out).map_err(|e| format!("{}: {e}", out.display()))? {
        let path = entry.map_err(|e| format!("{}: {e}", out.display()))?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if (name.starts_with("incremental-") || name.starts_with("interfaces-")) && path.is_dir() {
            fs::remove_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        } else if name == "points-timing.txt" {
            fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}
