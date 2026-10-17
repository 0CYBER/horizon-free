//! `horizon-port-pack`: port a Java modpack to Horizon (`docs/modpacks.md`).
//!
//! - `import <pack> <work>`: a `.mrpack`, CurseForge zip or instance folder
//!   to `<work>/pack.json`.
//! - `inventory <work> [--offline]`: Modrinth metadata (cached in
//!   `<work>/modrinth.json`), the jars in the instance or `<work>/jars/`,
//!   classes, tiers and dependency layers in `<work>/inventory.json` and
//!   `inventory.csv`.
//! - `carry <work> [--resources <prepared>] [<mod id>...]`: the content
//!   mods' data and assets into `<work>/mods/<id>/carry/`, a folder mod for
//!   local use. With the NeoForge oracle's `<work>/oracle/facts.json`
//!   (`tools/neoforge_oracle.py`), blocks and items come from its facts;
//!   `--resources` limits templates to the blocks Horizon's tables have.
//!
//! The work folder belongs to the user: jars and carried-over files stay
//! there and are never uploaded or committed.
mod carry;
mod classify;
mod import;
mod inventory;
mod jar;
mod modrinth;
mod oracle;

use std::{fs, path::Path, process::ExitCode};

const USAGE: &str = "usage: horizon-port-pack import <pack> <work> | inventory <work> [--offline] | carry <work> [--resources <prepared>] [<mod id>...]";

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
        Some("carry") => path(1).and_then(|w| {
            let mut rest = args[2..].to_vec();
            let resources = match rest.iter().position(|a| a == "--resources") {
                Some(i) if i + 1 < rest.len() => {
                    let r = rest.remove(i + 1);
                    rest.remove(i);
                    Some(std::path::PathBuf::from(r))
                }
                Some(_) => return Err(USAGE.to_owned()),
                None => None,
            };
            carry(w, resources.as_deref(), &rest)
        }),
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

/// The vanilla blocks a prepared resource root has (`facts.json`'s defaults).
fn vanilla_blocks(resources: &Path) -> Result<std::collections::BTreeSet<String>, String> {
    let facts: serde_json::Value = read_json(&resources.join("facts.json"))?;
    let defaults = facts["defaults"]
        .as_object()
        .ok_or("facts.json: no defaults")?;
    Ok(defaults.keys().cloned().collect())
}

fn carry(work: &Path, resources: Option<&Path>, only: &[String]) -> Result<(), String> {
    let records: Vec<inventory::Record> = read_json(&work.join("inventory.json"))?;
    let facts_path = work.join("oracle/facts.json");
    let facts = if facts_path.exists() {
        let f = oracle::Facts::read(&facts_path)?;
        println!(
            "oracle facts: {} blocks, {} items",
            f.blocks.len(),
            f.items.len()
        );
        Some(f)
    } else {
        None
    };
    let vanilla = resources.map(vanilla_blocks).transpose()?;
    let allowed = |b: &str| vanilla.as_ref().is_none_or(|v| v.contains(b));
    let mut done = 0;
    for r in &records {
        let chosen = if only.is_empty() {
            r.class == "content"
        } else {
            only.contains(&r.key)
        };
        let (Some(jar), Some(info)) = (&r.jar, &r.info) else {
            if chosen && !only.is_empty() {
                eprintln!("{}: no jar (put it in {}/jars/)", r.key, work.display());
            }
            continue;
        };
        if !chosen {
            continue;
        }
        match carry::carry(
            jar,
            info,
            &work.join("mods").join(&r.key),
            facts.as_ref(),
            &allowed,
        ) {
            Ok(report) => {
                done += 1;
                println!(
                    "{}: {} files, {} recipes upgraded ({} kept), {} loot tables upgraded, {} item definitions added, {} blocks ({} left out, {} with differences), {} items ({} left out), {} convention tags, {} outside",
                    r.key,
                    report.copied,
                    report.recipes_upgraded,
                    report.recipes_kept.len(),
                    report.loot_tables_upgraded,
                    report.item_definitions_added,
                    report.blocks.len(),
                    report.blocks_unknown.len(),
                    report.differences.len(),
                    report.items,
                    report.items_unknown.len(),
                    report.convention_tags,
                    report.outside.len()
                );
            }
            Err(e) => eprintln!("{}: {e}", r.key),
        }
    }
    println!(
        "{done} mods carried over into {}",
        work.join("mods").display()
    );
    Ok(())
}
