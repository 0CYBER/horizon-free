//! A 26.2 data pack's world generation, rewritten in 26.3's formats.
//!
//! 26.3 reworked the worldgen registries: configured carvers and features
//! became `carver` and `feature` with their `config` unwrapped, noise
//! parameters gained a base amplitude, density functions name their inputs
//! `input`/`left`/`right`, the noise settings moved aquifers, ore veins and
//! the surface rule out of the router into an `aquifers` object and a
//! `material_rule`, and biomes carry their mob spawns as attributes. Each
//! conversion here is the one vanilla's own data went through between 26.2
//! and 26.3, written so that the 26.3 pack generates what the 26.2 pack did:
//! the hard-coded parts of 26.2 (the beardifier added to the final density,
//! the deep dark aquifer exclusion, the ore veinifier, the interpolation
//! cell from `size_horizontal`/`size_vertical`) are spelled out as data.
//! Whatever has no faithful 26.3 form is kept as it is and listed in the
//! report.
mod noise;
mod registries;

use noise::noise;
pub use noise::parity_base_amplitude;
use registries::{biome, carver, dimension_type, structure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

/// 26.3's data pack format (`version.json`'s `pack_version.data_major`).
pub const PACK_FORMAT: u64 = 121;

/// What the upgrade did and what it could not do faithfully.
#[derive(Default, Debug)]
pub struct Report {
    /// Files rewritten, per registry.
    pub upgraded: BTreeMap<String, usize>,
    /// Files written that the 26.2 pack did not have (named density
    /// functions the noise settings share).
    pub added: Vec<String>,
    /// Parts that changed meaning or were dropped, with how often.
    pub lossy: BTreeMap<String, usize>,
    /// Types the upgrade does not know, left as they are.
    pub unknown: BTreeMap<String, usize>,
}

impl Report {
    fn lossy(&mut self, what: impl Into<String>) {
        *self.lossy.entry(what.into()).or_default() += 1;
    }
    fn unknown(&mut self, what: impl Into<String>) {
        *self.unknown.entry(what.into()).or_default() += 1;
    }
}

/// Every file of a pack, by its path inside the pack (`data/...`,
/// `pack.mcmeta`), upgraded. Files outside world generation are kept as
/// they are.
pub fn upgrade_pack(files: &BTreeMap<String, Vec<u8>>) -> (BTreeMap<String, Vec<u8>>, Report) {
    let mut report = Report::default();
    let mut json: BTreeMap<String, Value> = BTreeMap::new();
    let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (path, bytes) in files {
        let parsed = path
            .ends_with(".json")
            .then(|| serde_json::from_slice::<Value>(bytes).ok())
            .flatten();
        match parsed {
            Some(v) if path.starts_with("data/") => {
                json.insert(path.clone(), v);
            }
            _ => {
                out.insert(path.clone(), bytes.clone());
            }
        }
    }
    let mut written: BTreeMap<String, Value> = BTreeMap::new();
    for (path, v) in &json {
        let Some(file) = PackFile::parse(path) else {
            written.insert(path.clone(), v.clone());
            continue;
        };
        let upgraded = match file.registry.as_str() {
            "worldgen/noise" => Some(noise(v, &mut report)),
            "worldgen/configured_carver" => Some(carver(v, &mut report)),
            "worldgen/biome" => Some(biome(v, &mut report)),
            "worldgen/structure" => Some(structure(v)),
            "dimension_type" => Some(dimension_type(v)),
            "worldgen/template_pool" | "worldgen/processor_list" => {
                let mut v = v.clone();
                states(&mut v);
                Some(v)
            }
            _ => None,
        };
        let path = match file.registry.as_str() {
            "worldgen/configured_carver" => file.with_registry("worldgen/carver"),
            _ => path.clone(),
        };
        match upgraded {
            Some(u) => {
                *report.upgraded.entry(file.registry.clone()).or_default() += 1;
                written.insert(path, u);
            }
            None => {
                written.insert(path, v.clone());
            }
        }
    }
    for (path, v) in written {
        out.insert(path, pretty(&v));
    }
    if let Some(meta) = out
        .get("pack.mcmeta")
        .and_then(|b| serde_json::from_slice(b).ok())
    {
        out.insert("pack.mcmeta".into(), pretty(&mcmeta(meta)));
    }
    (out, report)
}

fn pretty(v: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(v).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// `pack.mcmeta` for 26.3: the pack format range is 26.3's alone.
fn mcmeta(mut meta: Value) -> Value {
    if let Some(pack) = meta.get_mut("pack").and_then(Value::as_object_mut) {
        pack.remove("supported_formats");
        pack.insert("pack_format".into(), json!(PACK_FORMAT));
        pack.insert("min_format".into(), json!(PACK_FORMAT));
        pack.insert("max_format".into(), json!(PACK_FORMAT));
        if let Some(Value::String(d)) = pack.get_mut("description") {
            d.push_str(" (upgraded to 26.3)");
        }
    }
    meta
}

/// `data/<ns>/<registry...>/<path>.json`, split by the registries the
/// upgrade knows (a registry name has one or two segments).
struct PackFile {
    namespace: String,
    registry: String,
    path: String,
}

impl PackFile {
    fn parse(path: &str) -> Option<Self> {
        let rest = path.strip_prefix("data/")?.strip_suffix(".json")?;
        let (namespace, rest) = rest.split_once('/')?;
        let (registry, path) = if let Some(r) = rest.strip_prefix("worldgen/") {
            let (kind, path) = r.split_once('/')?;
            (format!("worldgen/{kind}"), path)
        } else {
            rest.split_once('/').map(|(k, p)| (k.to_owned(), p))?
        };
        Some(Self {
            namespace: namespace.into(),
            registry,
            path: path.into(),
        })
    }
    fn with_registry(&self, registry: &str) -> String {
        format!("data/{}/{registry}/{}.json", self.namespace, self.path)
    }
}

/// A type name without its `minecraft:` prefix, as the codecs read both.
fn kind(v: &Value) -> Option<&str> {
    let t = v.get("type")?.as_str()?;
    Some(t.strip_prefix("minecraft:").unwrap_or(t))
}

fn vanilla(name: &str) -> Value {
    Value::String(format!("minecraft:{name}"))
}

// ---------------------------------------------------------------------------
// Block states

/// 26.2 writes a block or fluid state as `{"Name": ..., "Properties":
/// {...}}`; 26.3 reads `{"id": ..., "properties": {...}}` (or the id alone
/// for the default state).
fn states(v: &mut Value) {
    match v {
        Value::Object(o) => {
            let state = o.get("Name").is_some_and(Value::is_string)
                && o.keys().all(|k| k == "Name" || k == "Properties");
            if state {
                let name = o.remove("Name").unwrap_or_default();
                match o.remove("Properties") {
                    Some(Value::Object(p)) if !p.is_empty() => {
                        o.insert("id".into(), name);
                        o.insert("properties".into(), Value::Object(p));
                    }
                    _ => *v = name,
                }
                return;
            }
            o.values_mut().for_each(states);
        }
        Value::Array(a) => a.iter_mut().for_each(states),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Files

/// A pack's files by their path inside it, from a zip or a folder. A zip
/// whose `pack.mcmeta` sits in one top folder is read from that folder.
pub fn read_pack(path: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut files = BTreeMap::new();
    if path.is_dir() {
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).map_err(|e| at(&e))? {
                let p = entry.map_err(|e| at(&e))?.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(path) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    files.insert(rel, fs::read(&p).map_err(|e| at(&e))?);
                }
            }
        }
    } else {
        let file = fs::File::open(path).map_err(|e| at(&e))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| at(&e))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| at(&e))?;
            if entry.is_dir() {
                continue;
            }
            // A pack from the internet may name entries `../x` or `/x`.
            let name = entry
                .enclosed_name()
                .filter(|name| is_inside(name))
                .ok_or_else(|| at(&format!("entry {:?} leaves the pack", entry.name())))?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|e| at(&e))?;
            files.insert(name.to_string_lossy().replace('\\', "/"), bytes);
        }
    }
    if !files.contains_key("pack.mcmeta") {
        let top = files
            .keys()
            .find_map(|k| k.strip_suffix("/pack.mcmeta").filter(|t| !t.contains('/')))
            .map(|t| format!("{t}/"));
        if let Some(top) = top {
            files = files
                .into_iter()
                .filter_map(|(k, v)| k.strip_prefix(&top).map(|k| (k.to_owned(), v)))
                .collect();
        }
    }
    if !files.contains_key("pack.mcmeta") {
        return Err(format!("{}: no pack.mcmeta", path.display()));
    }
    Ok(files)
}

/// Upgrades the pack at `input` into the folder `out`, which must not
/// exist yet (a world's `datapacks/<name>`).
pub fn upgrade_into(input: &Path, out: &Path) -> Result<Report, String> {
    if out.exists() {
        return Err(format!("{} already exists", out.display()));
    }
    let files = read_pack(input)?;
    let (upgraded, report) = upgrade_pack(&files);
    for (path, bytes) in upgraded {
        if !is_inside(Path::new(&path)) {
            return Err(format!("{path}: leaves {}", out.display()));
        }
        let target = out.join(&path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(report)
}

/// Whether `path` is a plain relative path: no root, drive, `.` or `..`.
fn is_inside(path: &Path) -> bool {
    path.components().next().is_some()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

impl Report {
    /// The report as lines for the terminal.
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .upgraded
            .iter()
            .map(|(r, n)| format!("upgraded {n} {r}"))
            .collect();
        if !self.added.is_empty() {
            lines.push(format!(
                "added {} shared density functions",
                self.added.len()
            ));
        }
        lines.extend(
            self.lossy
                .iter()
                .map(|(w, n)| format!("not faithful ({n}x): {w}")),
        );
        lines.extend(
            self.unknown
                .iter()
                .map(|(w, n)| format!("left as it is ({n}x): {w}")),
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(path: &str, v: Value) -> (Value, Report) {
        let files = BTreeMap::from([(path.to_owned(), serde_json::to_vec(&v).unwrap())]);
        let (out, report) = upgrade_pack(&files);
        let (_, bytes) = out
            .into_iter()
            .find(|(p, _)| !p.contains("/upgraded/"))
            .unwrap();
        (serde_json::from_slice(&bytes).unwrap(), report)
    }

    #[test]
    fn noise_keeps_the_octaves_and_finds_the_parity_amplitude() {
        let (v, _) = up(
            "data/x/worldgen/noise/n.json",
            json!({"firstOctave": -7, "amplitudes": [1.0, 2.0, 1.0, 0.0, 0.0, 0.0]}),
        );
        // Vanilla 26.3's minecraft:ridge, which 26.2 wrote as above.
        assert_eq!(
            v,
            json!({"base_amplitude": 0.9147152149950137, "base_octave": -7, "octave_count": 6,
                "amplitude_modifiers": [1.0, 2.0, 1.0, 0.0, 0.0, 0.0]})
        );
        let (v, _) = up(
            "data/x/worldgen/noise/n.json",
            json!({"firstOctave": -3, "amplitudes": [1.0]}),
        );
        // Vanilla 26.3's minecraft:aquifer_barrier.
        assert_eq!(
            v,
            json!({"base_amplitude": 0.955388882960065, "base_octave": -3})
        );
    }

    #[test]
    fn biome_spawns_become_attributes() {
        let (v, _) = up(
            "data/x/worldgen/biome/b.json",
            json!({"spawners": {"monster": [{"type": "minecraft:zombie", "minCount": 4, "maxCount": 4,
                "weight": 95}], "ambient": []}, "spawn_costs": {}, "creature_spawn_probability": 0.07,
                "attributes": {"minecraft:visual/sky_color": "#78a7ff"}}),
        );
        assert_eq!(
            v["attributes"]["minecraft:gameplay/natural_mob_spawns"],
            json!({"modifier": "overlay", "argument": {"spawn_costs": {}, "spawns_by_category": {
                "monster": [{"type": "minecraft:zombie", "count": 4, "weight": 95}], "ambient": []}}})
        );
        assert_eq!(
            v["attributes"]["minecraft:gameplay/creature_world_gen_spawn_probability"],
            json!(0.07)
        );
        assert!(v.get("spawners").is_none());
    }

    #[test]
    fn entries_outside_the_pack_are_refused() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("upgrade-escape-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (i, name) in ["../escape.txt", "data/../../escape.txt", "/escape.txt"]
            .into_iter()
            .enumerate()
        {
            let zip_path = dir.join(format!("p{i}.zip"));
            let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
            for (file, bytes) in [("pack.mcmeta", &b"{}"[..]), (name, b"x")] {
                zip.start_file(file, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            let out = dir.join(format!("out{i}/datapacks/p-26.3"));
            let err = upgrade_into(&zip_path, &out).unwrap_err();
            assert!(err.contains("leaves the pack"), "{name}: {err}");
            assert!(!out.exists(), "{name}");
        }
        assert!(!dir.join("escape.txt").exists());
        assert!(is_inside(Path::new("data/x/worldgen/biome/b.json")));
        assert!(!is_inside(Path::new("data/../x.json")));
        assert!(!is_inside(Path::new("")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn carvers_move_and_spell_out_their_hard_coded_shape() {
        let files = BTreeMap::from([(
            "data/x/worldgen/configured_carver/c.json".to_owned(),
            serde_json::to_vec(&json!({"type": "minecraft:canyon", "config": {"probability": 0.01,
                "yScale": 3.0, "lava_level": {"above_bottom": 8}, "shape": {"width_smoothness": 3}}}))
            .unwrap(),
        )]);
        let (out, _) = upgrade_pack(&files);
        let v: Value = serde_json::from_slice(&out["data/x/worldgen/carver/c.json"]).unwrap();
        assert_eq!(
            v,
            json!({"type": "minecraft:canyon", "probability": 0.01,
                "shape": {"width_smoothness": 3, "y_scale": 3.0}})
        );
    }
}
