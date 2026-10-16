//! `horizon-port-pack`: port a Java modpack to Horizon (`docs/modpacks.md`).
//!
//! - `import <pack> <work>`: a `.mrpack`, CurseForge zip or instance folder
//!   to `<work>/pack.json`.
//! - `inventory <work> [--offline]`: Modrinth metadata (cached in
//!   `<work>/modrinth.json`), the jars in the instance or `<work>/jars/`,
//!   classes, tiers and dependency layers in `<work>/inventory.json` and
//!   `inventory.csv`.
//!
//! The work folder belongs to the user: jars stay there and are never
//! uploaded or committed.
mod classify;
mod import;
mod inventory;
mod jar;
mod modrinth;

use std::{fs, path::Path, process::ExitCode};

const USAGE: &str = "usage: horizon-port-pack import <pack> <work> | inventory <work> [--offline]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = |i: usize| args.get(i).map(Path::new).ok_or(USAGE.to_owned());
    let result = match args.first().map(String::as_str) {
        Some("import") => path(1)
            .and_then(|p| Ok((p, path(2)?)))
            .and_then(|(p, w)| import(p, w)),
        Some("inventory") => {
            path(1).and_then(|w| inventory(w, args.iter().any(|a| a == "--offline")))
        }
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn import(pack: &Path, work: &Path) -> Result<(), String> {
    let pack = import::import(pack)?;
    fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    write_json(&work.join("pack.json"), &pack)?;
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for f in &pack.files {
        *kinds.entry(f.kind()).or_default() += 1;
    }
    println!(
        "{} ({}): Minecraft {}, {} {}; files {kinds:?}; overrides {:?}",
        pack.name, pack.format, pack.minecraft, pack.loader, pack.loader_version, pack.overrides
    );
    Ok(())
}

fn inventory(work: &Path, offline: bool) -> Result<(), String> {
    let pack: import::Pack = read_json(&work.join("pack.json"))?;
    let cache_path = work.join("modrinth.json");
    let mut cache = modrinth::Cache::load(&cache_path);
    if !offline {
        let hashes: Vec<String> = pack.files.iter().filter_map(|f| f.sha1.clone()).collect();
        cache.fetch(&hashes)?;
        write_json(&cache_path, &cache)?;
    }
    let records = inventory::build(&pack, &cache, work);
    write_json(&work.join("inventory.json"), &records)?;
    fs::write(work.join("inventory.csv"), inventory::csv(&records))
        .map_err(|e| format!("inventory.csv: {e}"))?;
    let jars = records.iter().filter(|r| r.jar.is_some()).count();
    println!("{} files, {jars} jars read", records.len());
    for (title, field) in [
        (
            "class",
            (|r: &inventory::Record| r.class.as_str()) as fn(&inventory::Record) -> &str,
        ),
        ("tier", |r| r.tier.as_str()),
        ("confidence", |r| r.confidence.as_str()),
        ("source", |r| r.source.as_str()),
    ] {
        println!("{title}: {:?}", inventory::counts(&records, field));
    }
    let layers = records.iter().map(|r| r.layer).max().map_or(0, |m| m + 1);
    let per_layer: Vec<usize> = (0..layers)
        .map(|l| records.iter().filter(|r| r.layer == l).count())
        .collect();
    println!("layers: {per_layer:?}");
    let missing: usize = records.iter().map(|r| r.missing.len()).sum();
    if missing > 0 {
        println!("{missing} required dependencies are not in the pack (see inventory.json)");
    }
    Ok(())
}
