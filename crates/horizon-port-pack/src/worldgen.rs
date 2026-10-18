//! Structure mods' worldgen, carried over: the structure, placement and pool
//! element types of the common structure libraries rewritten as vanilla's,
//! and the loader's convention biome tags (`c:`) the structures name.
//!
//! The libraries' types add rules on top of vanilla's (a structure that may
//! not start in liquid, a minimum distance from the world origin, a count
//! cap per piece). Mapped to vanilla, those rules are lost: the structure
//! keeps its pieces and its spacing but can start where the mod would not.
//! Each rewrite is counted in the carry report, and every type left as it is
//! is listed, so the coverage report can say what generates and how closely.
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

/// Structure types that are vanilla's jigsaw with extra start rules.
const JIGSAWS: [&str; 3] = [
    "lithostitched:jigsaw",
    "moogs_structures:moogs_structures_generic_jigsaw_structure",
    "repurposed_structures:generic_jigsaw_structure",
];
/// Placements that are vanilla's random spread with extra rules.
const SPREADS: [&str; 2] = [
    "moogs_structures:advanced_random_spread",
    "repurposed_structures:advanced_random_spread",
];
/// Pool elements that are vanilla's with extra fields: (mod type, vanilla type).
const ELEMENTS: [(&str, &str); 3] = [
    (
        "moogs_structures:versioned_single_pool_element",
        "minecraft:single_pool_element",
    ),
    (
        "repurposed_structures:legacy_ocean_bottom_single_pool_element",
        "minecraft:legacy_single_pool_element",
    ),
    ("empty_pool_element", "minecraft:empty_pool_element"),
];

/// What carry-over did to a mod's worldgen files.
#[derive(Debug, Default, serde::Serialize)]
pub struct Upgrade {
    /// Files rewritten to vanilla types.
    pub mapped: usize,
    /// Types left as they are (Horizon skips what it cannot run), by type.
    pub unsupported: BTreeMap<String, usize>,
}

/// Rewrites one `data/<ns>/worldgen/<kind>/...` file in place; whether it
/// changed.
pub fn upgrade(kind: &str, v: &mut Value, report: &mut Upgrade) -> bool {
    let mut unsupported = |t: &str| *report.unsupported.entry(t.to_owned()).or_default() += 1;
    let changed = match kind {
        "structure" => match v["type"].as_str() {
            Some(t) if JIGSAWS.contains(&t) => {
                v["type"] = "minecraft:jigsaw".into();
                true
            }
            Some(t) if !t.starts_with("minecraft:") => {
                unsupported(t);
                false
            }
            _ => false,
        },
        "structure_set" => match v["placement"]["type"].as_str() {
            Some(t) if SPREADS.contains(&t) => {
                v["placement"]["type"] = "minecraft:random_spread".into();
                true
            }
            Some(t) if !t.starts_with("minecraft:") => {
                unsupported(t);
                false
            }
            _ => false,
        },
        "template_pool" => {
            let mut changed = false;
            let mut elements: Vec<&mut Value> = Vec::new();
            for e in v["elements"].as_array_mut().into_iter().flatten() {
                let element = &mut e["element"];
                // A list element holds elements of its own.
                if element.get("elements").is_some_and(Value::is_array) {
                    elements.extend(element["elements"].as_array_mut().into_iter().flatten());
                } else {
                    elements.push(element);
                }
            }
            for element in elements {
                let Some(t) = element["element_type"].as_str() else {
                    continue;
                };
                if let Some((_, to)) = ELEMENTS.iter().find(|(from, _)| *from == t) {
                    element["element_type"] = (*to).into();
                    changed = true;
                } else if !t.starts_with("minecraft:") {
                    unsupported(t);
                }
            }
            changed
        }
        _ => false,
    };
    report.mapped += usize::from(changed);
    changed
}

/// The `#c:` biome tags `v` names.
pub fn convention_tags(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => {
            if let Some(tag) = s.strip_prefix("#c:") {
                out.insert(tag.to_owned());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| convention_tags(x, out)),
        Value::Object(o) => o.values().for_each(|x| convention_tags(x, out)),
        _ => {}
    }
}

/// The loader's own convention biome tags (`data/c/tags/worldgen/biome/`),
/// by path, read from the loader jar `fetch` keeps in `<work>/loader/`.
pub fn loader_biome_tags(work: &Path) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Ok(dir) = std::fs::read_dir(work.join("loader")) else {
        return out;
    };
    for jar in dir.flatten().map(|e| e.path()) {
        let Some(mut zip) = std::fs::File::open(&jar)
            .ok()
            .and_then(|f| zip::ZipArchive::new(f).ok())
        else {
            continue;
        };
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else {
                continue;
            };
            let Some(path) = entry
                .name()
                .strip_prefix("data/c/tags/worldgen/biome/")
                .and_then(|p| p.strip_suffix(".json"))
                .map(str::to_owned)
            else {
                continue;
            };
            let mut text = String::new();
            if entry.read_to_string(&mut text).is_ok() {
                if let Ok(v) = serde_json::from_str(&text) {
                    out.insert(path, v);
                }
            }
        }
    }
    out
}

/// `tags` and the convention tags they include, in `conventions`.
pub fn with_included(
    tags: &BTreeSet<String>,
    conventions: &BTreeMap<String, Value>,
) -> BTreeSet<String> {
    let mut all = BTreeSet::new();
    let mut todo: Vec<String> = tags.iter().cloned().collect();
    while let Some(tag) = todo.pop() {
        if !conventions.contains_key(&tag) || !all.insert(tag.clone()) {
            continue;
        }
        let mut more = BTreeSet::new();
        convention_tags(&conventions[&tag], &mut more);
        todo.extend(more);
    }
    all
}

/// Where `fetch` keeps the pack's loader jar, and where it comes from:
/// NeoForge's own Maven repository. Only NeoForge packs need it today.
pub fn loader_jar(work: &Path, loader: &str, version: &str) -> Option<(PathBuf, String)> {
    (loader == "neoforge" && !version.is_empty()).then(|| {
        (
            work.join("loader").join(format!("neoforge-{version}-universal.jar")),
            format!(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-universal.jar"
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_library_types_to_vanillas() {
        let mut report = Upgrade::default();
        let mut s = json!({"type": "lithostitched:jigsaw", "start_pool": "a:b"});
        assert!(upgrade("structure", &mut s, &mut report));
        assert_eq!(s["type"], "minecraft:jigsaw");
        let mut nether =
            json!({"type": "moogs_structures:moogs_structures_generic_nether_jigsaw_structure"});
        assert!(!upgrade("structure", &mut nether, &mut report));
        let mut set = json!({"placement": {"type": "moogs_structures:advanced_random_spread",
            "spacing": 10, "separation": 5, "min_distance_from_world_origin": 250}});
        assert!(upgrade("structure_set", &mut set, &mut report));
        assert_eq!(set["placement"]["type"], "minecraft:random_spread");
        assert_eq!(set["placement"]["spacing"], 10);
        let mut pool = json!({"elements": [
            {"weight": 1, "element": {"element_type": "moogs_structures:versioned_single_pool_element", "location": "m:x"}},
            {"weight": 1, "element": {"element_type": "minecraft:list_pool_element", "elements": [
                {"element_type": "empty_pool_element"}]}},
            {"weight": 1, "element": {"element_type": "yungsapi:yung_single_element"}}
        ]});
        assert!(upgrade("template_pool", &mut pool, &mut report));
        assert_eq!(
            pool["elements"][0]["element"]["element_type"],
            "minecraft:single_pool_element"
        );
        assert_eq!(
            pool["elements"][1]["element"]["elements"][0]["element_type"],
            "minecraft:empty_pool_element"
        );
        assert_eq!(report.mapped, 3);
        assert_eq!(
            report.unsupported,
            BTreeMap::from([
                (
                    "moogs_structures:moogs_structures_generic_nether_jigsaw_structure".into(),
                    1
                ),
                ("yungsapi:yung_single_element".into(), 1),
            ])
        );
    }

    #[test]
    fn convention_tags_include_theirs() {
        let conventions = BTreeMap::from([
            (
                "is_hot".to_owned(),
                json!({"values": ["#c:is_desert", "#minecraft:is_badlands"]}),
            ),
            (
                "is_desert".to_owned(),
                json!({"values": ["minecraft:desert"]}),
            ),
            ("is_cold".to_owned(), json!({"values": []})),
        ]);
        let mut used = BTreeSet::new();
        convention_tags(
            &json!({"biomes": ["#c:is_hot", "#c:unknown", "#minecraft:x"]}),
            &mut used,
        );
        assert_eq!(used, BTreeSet::from(["is_hot".into(), "unknown".into()]));
        assert_eq!(
            with_included(&used, &conventions),
            BTreeSet::from(["is_hot".into(), "is_desert".into()])
        );
    }
}
