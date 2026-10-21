//! The steps on a work folder, as the command line and the launcher run
//! them. Each one reads what the earlier steps wrote and says what it did
//! through `log`, one line at a time.
use crate::{carry, fetch, import, inventory, modrinth, oracle, report};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

/// Where a step's lines go: the terminal, or the launcher's log.
pub type Log<'a> = &'a dyn Fn(String);

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Step 1: the pack at `pack` to `<work>/pack.json`.
pub fn import(pack: &Path, work: &Path, log: Log) -> Result<import::Pack, String> {
    let pack = import::import(pack)?;
    fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    write_json(&work.join("pack.json"), &pack)?;
    let mut kinds = BTreeMap::<&str, usize>::new();
    for f in &pack.files {
        *kinds.entry(f.kind()).or_default() += 1;
    }
    log(format!(
        "{} ({}): Minecraft {}, {} {}; files {kinds:?}; overrides {:?}",
        pack.name, pack.format, pack.minecraft, pack.loader, pack.loader_version, pack.overrides
    ));
    Ok(pack)
}

/// Step 2: `<work>/inventory.json` and `inventory.csv`. Online, Modrinth is
/// asked about the files it has not answered for yet; when it cannot be
/// reached, the cached answers and the reviewed table classify alone.
pub fn inventory(work: &Path, offline: bool, log: Log) -> Result<Vec<inventory::Record>, String> {
    let pack: import::Pack = read_json(&work.join("pack.json"))?;
    let cache_path = work.join("modrinth.json");
    let mut cache = modrinth::Cache::load(&cache_path);
    if !offline {
        let hashes: Vec<String> = pack.files.iter().filter_map(|f| f.sha1.clone()).collect();
        match cache.fetch(&hashes) {
            Ok(()) => write_json(&cache_path, &cache)?,
            Err(e) => log(format!("Modrinth not reached, classifying offline: {e}")),
        }
    }
    let records = inventory::build(&pack, &cache, work);
    write_json(&work.join("inventory.json"), &records)?;
    fs::write(work.join("inventory.csv"), inventory::csv(&records))
        .map_err(|e| format!("inventory.csv: {e}"))?;
    let jars = records.iter().filter(|r| r.jar.is_some()).count();
    log(format!("{} files, {jars} jars read", records.len()));
    for (title, field) in [
        (
            "class",
            (|r: &inventory::Record| r.class.as_str()) as fn(&inventory::Record) -> &str,
        ),
        ("tier", |r| r.tier.as_str()),
        ("confidence", |r| r.confidence.as_str()),
        ("source", |r| r.source.as_str()),
    ] {
        log(format!("{title}: {:?}", inventory::counts(&records, field)));
    }
    let layers = records.iter().map(|r| r.layer).max().map_or(0, |m| m + 1);
    let per_layer: Vec<usize> = (0..layers)
        .map(|l| records.iter().filter(|r| r.layer == l).count())
        .collect();
    log(format!("layers: {per_layer:?}"));
    let missing: usize = records.iter().map(|r| r.missing.len()).sum();
    if missing > 0 {
        log(format!(
            "{missing} required dependencies are not in the pack (see inventory.json)"
        ));
    }
    Ok(records)
}

/// The vanilla blocks a prepared resource root has (`facts.json`'s defaults).
fn vanilla_blocks(resources: &Path) -> Result<BTreeSet<String>, String> {
    let facts: serde_json::Value = read_json(&resources.join("facts.json"))?;
    let defaults = facts["defaults"]
        .as_object()
        .ok_or("facts.json: no defaults")?;
    Ok(defaults.keys().cloned().collect())
}

/// Step 4: the content mods with a jar (or the ones in `only`) carried over
/// into `<work>/mods/<id>/carry/`. Returns how many were.
pub fn carry(
    work: &Path,
    resources: Option<&Path>,
    only: &[String],
    log: Log,
) -> Result<usize, String> {
    let records: Vec<inventory::Record> = read_json(&work.join("inventory.json"))?;
    let facts_path = work.join("oracle/facts.json");
    let facts = if facts_path.exists() {
        let f = oracle::Facts::read(&facts_path)?;
        log(format!(
            "oracle facts: {} blocks, {} items",
            f.blocks.len(),
            f.items.len()
        ));
        Some(f)
    } else {
        None
    };
    // Without the prepared tables, templates are not limited to what the game has.
    let vanilla = resources.and_then(|r| {
        vanilla_blocks(r)
            .inspect_err(|e| log(format!("templates not limited to the game's blocks: {e}")))
            .ok()
    });
    let allowed = |b: &str| vanilla.as_ref().is_none_or(|v| v.contains(b));
    let conventions = crate::worldgen::loader_biome_tags(work);
    let chosen = |r: &inventory::Record| {
        if only.is_empty() {
            r.class == "content"
        } else {
            only.contains(&r.key)
        }
    };
    // The mods ported along (carried now or before), for the conditions of
    // the mods' worldgen modifiers.
    let ported: BTreeSet<&str> = records
        .iter()
        .filter(|r| {
            r.jar.is_some() && (chosen(r) || work.join("mods").join(&r.key).join("carry").is_dir())
        })
        .filter_map(|r| Some(r.info.as_ref()?.mod_id.as_str()))
        .chain(["minecraft"])
        .collect();
    let loaded = |m: &str| ported.contains(m);
    // Every mod of the pack, ported or not: a namespace one of them has as
    // its id is never another mod's.
    let in_pack: BTreeSet<&str> = records
        .iter()
        .flat_map(|r| {
            [
                Some(r.key.as_str()),
                r.info.as_ref().map(|i| i.mod_id.as_str()),
            ]
        })
        .flatten()
        .collect();
    let in_pack = |m: &str| in_pack.contains(m);
    let pack = carry::Pack {
        loaded: &loaded,
        mods: &in_pack,
    };
    let mut done = 0;
    for r in &records {
        let chosen = chosen(r);
        let (Some(jar), Some(info)) = (&r.jar, &r.info) else {
            if chosen && !only.is_empty() {
                log(format!(
                    "{}: no jar (put it in {}/jars/)",
                    r.key,
                    work.display()
                ));
            }
            continue;
        };
        if !chosen {
            continue;
        }
        // The oracle's facts for the mods it ran; a mod it did not load (a
        // library missing) keeps the guesses from its blockstates.
        let prefix = format!("{}:", info.mod_id);
        let seen = |f: &&oracle::Facts| {
            f.blocks
                .keys()
                .chain(f.items.keys())
                .any(|k| k.starts_with(&prefix))
        };
        match carry::carry(
            jar,
            info,
            &work.join("mods").join(&r.key),
            facts.as_ref().filter(seen),
            &allowed,
            &pack,
            &conventions,
        ) {
            Ok(report) => {
                done += 1;
                log(format!(
                    "{}: {} files, {} recipes upgraded ({} kept), {} loot tables upgraded ({} added), {} item definitions added, {} blocks ({} left out, {} with differences), {} items ({} left out), {} convention tags, {} outside",
                    r.key,
                    report.copied,
                    report.recipes_upgraded,
                    report.recipes_kept.len(),
                    report.loot_tables_upgraded,
                    report.loot_tables_added,
                    report.item_definitions_added,
                    report.blocks.len(),
                    report.blocks_unknown.len(),
                    report.differences.len(),
                    report.items,
                    report.items_unknown.len(),
                    report.convention_tags,
                    report.outside.len()
                ));
            }
            Err(e) => log(format!("{}: {e}", r.key)),
        }
    }
    log(format!(
        "{done} mods carried over into {}",
        work.join("mods").display()
    ));
    Ok(done)
}

/// The pack's title as the report heads it.
pub fn title(pack: &import::Pack) -> String {
    format!(
        "{} ({} {}, Minecraft {})",
        pack.name, pack.loader, pack.loader_version, pack.minecraft
    )
}

/// Step 7: `<work>/coverage.json` and `coverage.md`.
pub fn report(work: &Path, log: Log) -> Result<Vec<report::Row>, String> {
    let pack: import::Pack = read_json(&work.join("pack.json"))?;
    let records: Vec<inventory::Record> = read_json(&work.join("inventory.json"))?;
    let rows = report::build(&records, work);
    write_json(&work.join("coverage.json"), &rows)?;
    let title = title(&pack);
    fs::write(work.join("coverage.md"), report::markdown(&title, &rows))
        .map_err(|e| format!("coverage.md: {e}"))?;
    log(format!(
        "{title}: {} files: {:?}",
        rows.len(),
        report::counts(&rows)
    ));
    Ok(rows)
}

/// What a player's machine can port today without an agent: content mods
/// of the data tier (carry-over only).
pub fn portable(r: &inventory::Record) -> bool {
    r.class == "content" && r.tier == "data"
}

/// Everything after the import, for the portable mods: their jars (`fetch`,
/// from `from` when the player has them), the inventory again with the jars
/// read, the carry-over and the report. `resources` is the prepared
/// version the ports will play on.
pub fn port(
    work: &Path,
    resources: Option<&Path>,
    from: Option<&Path>,
    log: Log,
) -> Result<(fetch::Fetched, Vec<report::Row>), String> {
    let records = match read_json::<Vec<inventory::Record>>(&work.join("inventory.json")) {
        Ok(r) => r,
        Err(_) => inventory(work, false, log)?,
    };
    let fetched = fetch::fetch(work, &records, from, &portable, log)?;
    // The jars tell classes and tiers better than metadata alone.
    let records = inventory(work, true, log)?;
    let only: Vec<String> = records
        .iter()
        .filter(|r| portable(r) && r.jar.is_some())
        .map(|r| r.key.clone())
        .collect();
    if only.is_empty() {
        log("no portable mod has its jar: nothing to carry over".into());
    } else {
        carry(work, resources, &only, log)?;
    }
    Ok((fetched, report(work, log)?))
}
