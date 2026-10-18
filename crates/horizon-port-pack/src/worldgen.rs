//! Structure mods' worldgen, carried over: block and fluid states in 26.3's
//! shape, the structure, placement and pool element types of the common
//! structure libraries rewritten as vanilla's, and the loader's convention
//! biome tags (`c:`) the structures name.
//!
//! The libraries' types add rules on top of vanilla's. The start and
//! placement rules of Repurposed Structures' and Moog's jigsaws (no start in
//! liquid, biomes and terrain height in a radius, a minimum distance from
//! the world origin, sets to keep away from) are written under `horizon:`
//! keys the game reads (`horizon_world::worldgen::structure::start_checks`),
//! so those structures start where the mod would. Rules that change how the
//! pieces are laid out (piece caps, burying, clipping) are lost and counted
//! by name. Each rewrite is counted in the carry report, and every type left
//! as it is is listed, so the coverage report can say what generates and how
//! closely.
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
/// YUNG's placements: vanilla's random spread with an enhanced exclusion
/// zone (several sets), plus, for the temples, no start near an ocean or a
/// river, and for strongholds, starts only in odd rings around the origin.
const YUNG_SPREADS: [&str; 4] = [
    "yungsapi:enhanced_random_spread",
    "betterdeserttemples:desert_temple",
    "betterjungletemples:jungle_temple",
    "betterstrongholds:stronghold",
];
/// YUNG's structures and the vanilla structure type each one's mod turns
/// off (its DisableVanilla*Mixin, on by default in each mod's config).
const YUNG_REPLACES: [(&str, &str); 7] = [
    (
        "betterdeserttemples:desert_temple",
        "minecraft:desert_pyramid",
    ),
    (
        "betterjungletemples:jungle_temple",
        "minecraft:jungle_temple",
    ),
    ("betterwitchhuts:witch_hut", "minecraft:swamp_hut"),
    (
        "betteroceanmonuments:ocean_monument",
        "minecraft:ocean_monument",
    ),
    ("betterstrongholds:stronghold", "minecraft:stronghold"),
    ("betterfortresses:fortress", "minecraft:fortress"),
    ("bettermineshafts:mineshaft_", "minecraft:mineshaft"),
];
/// Moog's Structure Lib spreads its structures 1.65 times as far apart as
/// its files say (AdvancedRandomSpread's constructor).
const MOOGS_SPACING: f64 = 1.65;
/// The jigsaws' start rules and the `horizon:start_checks` key each maps to.
const START_RULES: [(&str, &str); 3] = [
    ("valid_biome_radius_check", "biome_radius"),
    ("terrain_height_radius_check", "terrain_radius"),
    ("allowed_terrain_height_range", "terrain_range"),
];
/// The jigsaws' rules on laying out pieces, which vanilla's jigsaw has no
/// place for.
const LAYOUT_RULES: [&str; 6] = [
    "y_allowance",
    "burying_type",
    "use_bounding_box_hack",
    "pools_that_ignore_boundaries",
    "enhanced_terrain_adaptation",
    "vertical_distance_from_start_piece",
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
    /// Files whose block and fluid states were written in 26.3's shape.
    pub block_states: usize,
    /// Types left as they are (Horizon skips what it cannot run), by type.
    pub unsupported: BTreeMap<String, usize>,
    /// Rules of mapped types the game does not run, by field.
    pub rules_lost: BTreeMap<String, usize>,
    /// Processors written in Java, taken out of their lists, by type.
    pub processors_dropped: BTreeMap<String, usize>,
    /// Entries added to other structure sets and template pools
    /// (`horizon:add_to`), by the set or pool they join.
    pub added_to: BTreeMap<String, usize>,
    /// Lithostitched worldgen modifiers not carried: by type, `conditions`
    /// when they need a mod that is not ported along, `target` when they
    /// name their sets or pools by a tag.
    pub modifiers_left: BTreeMap<String, usize>,
}

/// Where a mod's additions to other sets and pools go in its carry:
/// `data/<id>/worldgen/<kind>/horizon_added/<name>.json`.
pub const ADDED: &str = "horizon_added";

/// A lithostitched worldgen modifier
/// (`data/<ns>/lithostitched/worldgen_modifier/`) that adds to structure
/// sets or template pools, as a file the game merges into them (the key
/// `horizon:add_to`, `horizon_client::mod_content`): (kind, file). The
/// modifier's conditions are checked against `loaded`, the mods ported
/// along: a compat addition for a mod that is not would place that mod's
/// blocks, which the game does not have. Other modifier types are left out.
/// Both are counted in `report`.
pub fn modifier(
    v: &Value,
    loaded: &dyn Fn(&str) -> bool,
    report: &mut Upgrade,
) -> Option<(&'static str, Value)> {
    let t = v["type"].as_str().unwrap_or_default();
    let mut left = |why: &str| *report.modifiers_left.entry(why.to_owned()).or_default() += 1;
    let (kind, targets, from, to) = match t {
        "lithostitched:add_structure_set_entries" => {
            ("structure_set", "structure_sets", "entries", "structures")
        }
        "lithostitched:add_template_pool_elements" => {
            ("template_pool", "template_pools", "elements", "elements")
        }
        _ => {
            left(t);
            return None;
        }
    };
    if !conditions_hold(v, loaded) {
        left("conditions");
        return None;
    }
    let targets: Option<Vec<&str>> = match &v[targets] {
        Value::String(s) => Some(vec![s.as_str()]),
        Value::Array(a) => a.iter().map(Value::as_str).collect(),
        _ => None,
    };
    let Some(targets) = targets.filter(|t| !t.is_empty() && t.iter().all(|t| !t.starts_with('#')))
    else {
        left("target");
        return None;
    };
    let entries = v[from].as_array().cloned().unwrap_or_default();
    for target in &targets {
        *report.added_to.entry((*target).to_owned()).or_default() += entries.len();
    }
    Some((
        kind,
        serde_json::json!({"horizon:add_to": targets, to: entries}),
    ))
}

/// Whether a modifier's load conditions hold: lithostitched's `predicate`,
/// and the loader's conditions (NeoForge's, else Fabric's). A condition
/// type not known here does not hold.
fn conditions_hold(v: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
    let all = |list: &Value, test: &dyn Fn(&Value) -> bool| {
        list.as_array().is_some_and(|a| a.iter().all(test))
    };
    fn predicate(p: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
        let test = |q: &Value| predicate(q, loaded);
        let list = &p["predicates"];
        match p["type"]
            .as_str()
            .map(|t| t.trim_start_matches("lithostitched:"))
        {
            Some("true") => true,
            Some("mod_loaded") => p["mod_id"].as_str().is_some_and(loaded),
            Some("not") => !predicate(&p["predicate"], loaded),
            Some("all_of") => list.as_array().is_some_and(|a| a.iter().all(test)),
            Some("any_of") => list.as_array().is_some_and(|a| a.iter().any(test)),
            _ => false,
        }
    }
    fn neoforge(c: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
        let test = |q: &Value| neoforge(q, loaded);
        match c["type"].as_str() {
            Some("neoforge:true") => true,
            Some("neoforge:mod_loaded") => c["modid"].as_str().is_some_and(loaded),
            Some("neoforge:not") => !neoforge(&c["value"], loaded),
            Some("neoforge:and") => c["values"].as_array().is_some_and(|a| a.iter().all(test)),
            Some("neoforge:or") => c["values"].as_array().is_some_and(|a| a.iter().any(test)),
            _ => false,
        }
    }
    fn fabric(c: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
        let test = |q: &Value| fabric(q, loaded);
        let mods = || {
            c["values"]
                .as_array()
                .into_iter()
                .flatten()
                .map(Value::as_str)
        };
        match c["condition"].as_str() {
            Some("fabric:true") => true,
            Some("fabric:all_mods_loaded") => mods().all(|m| m.is_some_and(loaded)),
            Some("fabric:any_mods_loaded") => mods().any(|m| m.is_some_and(loaded)),
            Some("fabric:not") => !fabric(&c["value"], loaded),
            Some("fabric:and") => c["values"].as_array().is_some_and(|a| a.iter().all(test)),
            Some("fabric:or") => c["values"].as_array().is_some_and(|a| a.iter().any(test)),
            _ => false,
        }
    }
    let litho = v.get("predicate").is_none_or(|p| predicate(p, loaded));
    let loader = match (
        v.get("neoforge:conditions"),
        v.get("fabric:load_conditions"),
    ) {
        (Some(c), _) => all(c, &|c| neoforge(c, loaded)),
        // One condition or a list of them.
        (None, Some(c)) if c.is_array() => all(c, &|c| fabric(c, loaded)),
        (None, Some(c)) => fabric(c, loaded),
        (None, None) => true,
    };
    litho && loader
}

/// ChoiceTheorem's Overhauled Village's villages and outposts, which it adds
/// to vanilla's `villages` and `pillager_outposts` sets in Java code
/// (`CTOV.registerstructure`, through lithostitched), with its default
/// config: every village type in three sizes, weighted 10, 4 and 1, and
/// every outpost weighted 1. Vanilla's own entries stay (weight 1 each).
const CTOV_VILLAGES: [&str; 21] = [
    "beach",
    "christmas",
    "desert",
    "desert_oasis",
    "dark_forest",
    "jungle",
    "jungle_tree",
    "mesa",
    "mesa_fortified",
    "mountain",
    "mountain_alpine",
    "mushroom",
    "plains",
    "plains_fortified",
    "savanna",
    "savanna_na",
    "snowy_igloo",
    "swamp",
    "swamp_fortified",
    "taiga",
    "taiga_fortified",
];
const CTOV_OUTPOSTS: [&str; 11] = [
    "beach",
    "dark_forest",
    "desert",
    "jungle",
    "badlands",
    "mountain",
    "plains",
    "savanna",
    "snowy",
    "swamp",
    "taiga",
];

/// The entries a mod adds to other structure sets in Java code, as its
/// default config has them: (file name, set, `[(structure, weight)]`).
/// Lithostitched appends them in the order of a hash map of every modifier
/// in the pack, which carry cannot know; they are written in the order the
/// mod registers them. Each set picks by weight, so how often each starts
/// is the same.
pub fn code_set_entries(mod_id: &str) -> Vec<(&'static str, &'static str, Vec<(String, u32)>)> {
    match mod_id {
        "ctov" => {
            let outposts = CTOV_OUTPOSTS
                .iter()
                .map(|v| (format!("ctov:pillager_outpost_{v}"), 1))
                .collect();
            let villages = [("small", 10), ("medium", 4), ("large", 1)]
                .into_iter()
                .flat_map(|(size, weight)| {
                    CTOV_VILLAGES
                        .iter()
                        .map(move |v| (format!("ctov:{size}/village_{v}"), weight))
                })
                .collect();
            vec![
                ("pillager_outposts", "minecraft:pillager_outposts", outposts),
                ("villages", "minecraft:villages", villages),
            ]
        }
        _ => Vec::new(),
    }
}

/// Rewrites one `data/<ns>/worldgen/<kind>/...` file in place; whether it
/// changed.
/// `set_tags` are the mod's structure set tags, expanded
/// ([`structure_set_tags`]).
pub fn upgrade(
    kind: &str,
    v: &mut Value,
    report: &mut Upgrade,
    set_tags: &BTreeMap<String, Vec<String>>,
) -> bool {
    let states = block_states(v);
    report.block_states += usize::from(states);
    let dropped = match kind {
        "processor_list" => drop_java_processors(v, &mut report.processors_dropped),
        "template_pool" => {
            let mut changed = false;
            for e in v["elements"].as_array_mut().into_iter().flatten() {
                let element = &mut e["element"];
                if let Some(list) = element.get_mut("elements").and_then(Value::as_array_mut) {
                    for inner in list {
                        if let Some(p) = inner.get_mut("processors") {
                            changed |= drop_java_processors(p, &mut report.processors_dropped);
                        }
                    }
                } else if let Some(p) = element.get_mut("processors") {
                    changed |= drop_java_processors(p, &mut report.processors_dropped);
                }
            }
            changed
        }
        _ => false,
    };
    upgrade_types(kind, v, report, set_tags) || states || dropped
}

/// Takes the processors written in Java (any type outside `minecraft:`)
/// out of a processor list: `{"processors": [...]}`, a bare list, or an id
/// (left as it is). The game, like vanilla, refuses a list holding a type it
/// does not know, and with it every pool that names the list, so without
/// this the structure does not generate at all. Without them it generates
/// undecorated: no pillars under it, spawners left as the template has
/// them, blocks not swapped. Whether anything was taken out.
fn drop_java_processors(list: &mut Value, dropped: &mut BTreeMap<String, usize>) -> bool {
    let processors = match list {
        Value::Array(a) => a,
        Value::Object(o) => match o.get_mut("processors") {
            Some(Value::Array(a)) => a,
            _ => return false,
        },
        _ => return false,
    };
    let before = processors.len();
    processors.retain(|p| {
        let t = p["processor_type"].as_str().unwrap_or_default();
        let java = !t.starts_with("minecraft:");
        if java {
            *dropped.entry(t.to_owned()).or_default() += 1;
        }
        !java
    });
    processors.len() != before
}

/// 1.21.1 writes a block or fluid state as `{"Name": ..., "Properties":
/// {...}}`; 26.3's codec reads `{"id": ..., "properties": {...}}` and
/// refuses the old shape, which fails the processor list, feature or pool
/// holding it. Whether anything changed.
fn block_states(v: &mut Value) -> bool {
    match v {
        Value::Object(o) => {
            let state = o.get("Name").is_some_and(Value::is_string)
                && o.keys().all(|k| k == "Name" || k == "Properties");
            if state {
                if let Some(name) = o.remove("Name") {
                    o.insert("id".into(), name);
                }
                if let Some(properties) = o.remove("Properties") {
                    o.insert("properties".into(), properties);
                }
                return true;
            }
            o.values_mut()
                .fold(false, |changed, x| block_states(x) | changed)
        }
        Value::Array(a) => a
            .iter_mut()
            .fold(false, |changed, x| block_states(x) | changed),
        _ => false,
    }
}

/// The library types of one file rewritten as vanilla's; whether it changed.
fn upgrade_types(
    kind: &str,
    v: &mut Value,
    report: &mut Upgrade,
    set_tags: &BTreeMap<String, Vec<String>>,
) -> bool {
    let lost = &mut report.rules_lost;
    let mut unsupported = |t: &str| *report.unsupported.entry(t.to_owned()).or_default() += 1;
    let changed = match kind {
        "structure" => match v["type"].as_str() {
            Some("yungsapi:yung_jigsaw") => {
                yung_structure(v, lost);
                true
            }
            Some(t) if JIGSAWS.contains(&t) => {
                let liquid = match t.split_once(':').map(|(ns, _)| ns) {
                    Some("moogs_structures") => "top",
                    _ => "lowest",
                };
                start_checks(v, liquid, lost);
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
                let moogs = t.starts_with("moogs_structures:");
                let p = &mut v["placement"];
                if moogs {
                    moogs_spacing(p, lost);
                }
                placement_rules(p, set_tags, lost);
                p["type"] = "minecraft:random_spread".into();
                true
            }
            Some(t) if YUNG_SPREADS.contains(&t) => {
                let t = t.to_owned();
                yung_placement(&t, &mut v["placement"], set_tags, lost);
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
                } else if t == "yungsapi:yung_single_element" {
                    yung_element(element, lost);
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

/// YUNG's API's jigsaw as vanilla's, laid out by its own assembler in the
/// game (`horizon:yungsapi`, `horizon_world::worldgen::structure::yung`):
/// the heights pieces stay within move under the key. Its terrain
/// adaptation and a start offset in the chunk are lost.
fn yung_structure(v: &mut Value, lost: &mut BTreeMap<String, usize>) {
    let Some(o) = v.as_object_mut() else {
        return;
    };
    o.insert("type".into(), "minecraft:jigsaw".into());
    let mut rules = serde_json::Map::new();
    for key in ["min_y", "max_y"] {
        if let Some(y) = o.remove(key) {
            rules.insert(key.into(), y);
        }
    }
    let none = |a: &Value| a["type"] == "yungsapi:none";
    if o.remove("enhanced_terrain_adaptation")
        .is_some_and(|a| !none(&a))
    {
        *lost
            .entry("enhanced_terrain_adaptation".into())
            .or_default() += 1;
    }
    for key in ["x_offset_in_chunk", "z_offset_in_chunk"] {
        if o.remove(key).is_some_and(|n| n != 0) {
            *lost.entry(key.into()).or_default() += 1;
        }
    }
    o.insert("horizon:yungsapi".into(), rules.into());
}

/// YUNG's single pool element as vanilla's, its rules under
/// `horizon:yungsapi`: name and `max_count`, depth range, priority,
/// `ignore_bounds` and its condition. Its terrain adaptation, dead-end pool
/// and modifiers are lost.
fn yung_element(element: &mut Value, lost: &mut BTreeMap<String, usize>) {
    let Some(o) = element.as_object_mut() else {
        return;
    };
    o.insert(
        "element_type".into(),
        "minecraft:single_pool_element".into(),
    );
    let mut rules = serde_json::Map::new();
    for (from, to) in [
        ("name", "name"),
        ("max_count", "max_count"),
        ("min_required_depth", "min_depth"),
        ("max_possible_depth", "max_depth"),
        ("is_priority", "priority"),
        ("ignore_bounds", "ignore_bounds"),
    ] {
        if let Some(x) = o.remove(from) {
            rules.insert(to.into(), x);
        }
    }
    if let Some(c) = o.remove("condition") {
        rules.insert("condition".into(), yung_condition(&c, lost));
    }
    for key in ["enhanced_terrain_adaptation", "deadend_pool", "modifiers"] {
        let empty =
            |x: &Value| x.as_array().is_some_and(Vec::is_empty) || x["type"] == "yungsapi:none";
        if o.remove(key).is_some_and(|x| !empty(&x)) {
            *lost.entry(format!("yungsapi {key}")).or_default() += 1;
        }
    }
    o.insert("horizon:yungsapi".into(), rules.into());
}

/// One of YUNG's structure conditions in the game's terms. `mod_loaded`
/// holds for Minecraft alone: a mod that needs code is not ported, and the
/// pieces it guards would place its blocks. A condition the game does not
/// have does not hold, and is counted.
fn yung_condition(c: &Value, lost: &mut BTreeMap<String, usize>) -> Value {
    let t = c["type"].as_str().unwrap_or_default();
    let list = |lost: &mut BTreeMap<String, usize>| -> Value {
        let all = c["conditions"].as_array().into_iter().flatten();
        all.map(|x| yung_condition(x, lost))
            .collect::<Vec<_>>()
            .into()
    };
    let opt = |k: &str| c.get(k).cloned().unwrap_or(Value::Null);
    match t.trim_start_matches("yungsapi:") {
        "always_true" => serde_json::json!({"type": "true"}),
        "not" => {
            serde_json::json!({"type": "not", "condition": yung_condition(&c["condition"], lost)})
        }
        "all_of" => serde_json::json!({"type": "all_of", "conditions": list(lost)}),
        "any_of" => serde_json::json!({"type": "any_of", "conditions": list(lost)}),
        "depth" => serde_json::json!({"type": "depth",
            "min_depth": opt("min_required_depth"), "max_depth": opt("max_possible_depth")}),
        "altitude" => serde_json::json!({"type": "altitude",
            "bottom": opt("bottom_cutoff_y"), "top": opt("top_cutoff_y")}),
        "random_chance" => serde_json::json!({"type": "random_chance", "chance": opt("chance")}),
        "rotation" => serde_json::json!({"type": "rotation", "rotations": opt("rotations")}),
        "piece_in_horizontal_direction" => {
            // "yungsapi:all" (or no list) matches any piece.
            let pieces: Vec<Value> = c["pieces"].as_array().cloned().unwrap_or_default();
            let any = pieces.is_empty() || pieces.iter().any(|p| p == "yungsapi:all");
            serde_json::json!({"type": "piece_in_horizontal_direction",
                "pieces": if any { Vec::new() } else { pieces },
                "range": opt("range"), "rotation": opt("rotation")})
        }
        "mod_loaded" if c["modid"] == "minecraft" => serde_json::json!({"type": "true"}),
        "mod_loaded" => serde_json::json!({"type": "false"}),
        _ => {
            *lost.entry(format!("yungsapi condition {t}")).or_default() += 1;
            serde_json::json!({"type": "false"})
        }
    }
}

/// Moves a Repurposed Structures or Moog's jigsaw's start rules under
/// `horizon:start_checks`, and counts the layout rules it loses. `liquid` is
/// the block its no-start-in-liquid rule reads.
fn start_checks(v: &mut Value, liquid: &str, lost: &mut BTreeMap<String, usize>) {
    let Some(o) = v.as_object_mut() else {
        return;
    };
    let mut checks = serde_json::Map::new();
    for (from, to) in START_RULES {
        if let Some(n) = o.remove(from) {
            checks.insert(to.into(), n);
        }
    }
    if o.remove("cannot_spawn_in_liquid").and_then(|b| b.as_bool()) == Some(true) {
        checks.insert("liquid".into(), liquid.into());
    }
    // y_allowance bounds the terrain heights checked; it also clips pieces,
    // which is lost.
    for (from, to) in [("min_y_allowed", "min_y"), ("max_y_allowed", "max_y")] {
        if let Some(n) = o.get("y_allowance").and_then(|y| y.get(from)) {
            checks.insert(to.into(), n.clone());
        }
    }
    for rule in LAYOUT_RULES {
        if o.remove(rule).is_some_and(|r| r != Value::Bool(false)) {
            *lost.entry(rule.into()).or_default() += 1;
        }
    }
    if !checks.is_empty() {
        o.insert("horizon:start_checks".into(), checks.into());
    }
}

/// AdvancedRandomSpread's spacing and separation, as Moog's computes them
/// with its default config: 1.65 times the file's, rounded, the separation
/// below the spacing. `when_replacing` (other numbers while a preset
/// replaces a vanilla structure) is lost.
fn moogs_spacing(p: &mut Value, lost: &mut BTreeMap<String, usize>) {
    let scaled = |k: &str| (p[k].as_f64().unwrap_or(0.) * MOOGS_SPACING).round() as i64;
    let spacing = scaled("spacing").max(1);
    let separation = scaled("separation").min(spacing - 1);
    p["spacing"] = spacing.into();
    p["separation"] = separation.into();
    if let Some(o) = p.as_object_mut() {
        if o.remove("when_replacing").is_some() {
            *lost.entry("when_replacing".into()).or_default() += 1;
        }
    }
}

/// An advanced random spread's minimum distance from the world origin and
/// its super exclusion zone (sets named by a tag) under `horizon:` keys.
fn placement_rules(
    p: &mut Value,
    set_tags: &BTreeMap<String, Vec<String>>,
    lost: &mut BTreeMap<String, usize>,
) {
    let Some(o) = p.as_object_mut() else {
        return;
    };
    if let Some(d) = o.remove("min_distance_from_world_origin") {
        o.insert("horizon:min_distance_from_world_origin".into(), d);
    }
    let zone = ["super_exclusion_zone", "enhanced_exclusion_zone"]
        .into_iter()
        .find_map(|k| Some((k, o.remove(k)?)));
    if let Some((key, zone)) = zone {
        let sets: Vec<String> = match &zone["other_set"] {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => set_tags.get(tag).cloned().unwrap_or_default(),
                None => vec![s.clone()],
            },
            Value::Array(a) => a
                .iter()
                .filter_map(|s| Some(s.as_str()?.to_owned()))
                .collect(),
            _ => Vec::new(),
        };
        if sets.is_empty() {
            *lost.entry(key.into()).or_default() += 1;
        } else {
            o.insert(
                "horizon:exclusion_zone".into(),
                serde_json::json!({"other_sets": sets, "chunk_count": zone["chunk_count"]}),
            );
        }
    }
}

/// One of YUNG's placements as vanilla's random spread with `horizon:` keys:
/// the enhanced exclusion zone as `horizon:exclusion_zone`; the temples'
/// ocean and river search (findBiomeHorizontal around the chunk's middle
/// block at y 120, 48 blocks, every 2 quarts) as `horizon:avoid_biomes`; the
/// strongholds' rings as `horizon:ring_sections`.
fn yung_placement(
    t: &str,
    p: &mut Value,
    set_tags: &BTreeMap<String, Vec<String>>,
    lost: &mut BTreeMap<String, usize>,
) {
    placement_rules(p, set_tags, lost);
    p["type"] = "minecraft:random_spread".into();
    let Some(o) = p.as_object_mut() else {
        return;
    };
    match t {
        "betterdeserttemples:desert_temple" | "betterjungletemples:jungle_temple" => {
            o.insert(
                "horizon:avoid_biomes".into(),
                serde_json::json!({"biomes": ["#minecraft:is_river", "#minecraft:is_ocean"],
                    "radius": 48, "step": 2, "y": 120}),
            );
        }
        "betterstrongholds:stronghold" => {
            let mut rings = serde_json::Map::new();
            for (from, to) in [
                ("chunk_distance_to_first_ring", "distance_to_first_ring"),
                ("ring_chunk_thickness", "thickness"),
                ("max_ring_section", "max_section"),
            ] {
                if let Some(n) = o.remove(from) {
                    rings.insert(to.into(), n);
                }
            }
            o.insert("horizon:ring_sections".into(), rings.into());
        }
        _ => {}
    }
}

/// Marks a YUNG's structure that the game can start (its type is vanilla's
/// after the upgrade) with the vanilla type its mod turns off, under
/// `horizon:disables`. One the game cannot start leaves the vanilla
/// structure in place, so the world keeps one of the two.
pub fn disables(id: &str, v: &mut Value) -> bool {
    let Some((_, vanilla)) = YUNG_REPLACES.iter().find(|(s, _)| id.starts_with(s)) else {
        return false;
    };
    if !v["type"]
        .as_str()
        .is_some_and(|t| t.starts_with("minecraft:"))
    {
        return false;
    }
    v["horizon:disables"] = serde_json::json!([vanilla]);
    true
}

/// The mod's structure set tags (`data/<ns>/tags/worldgen/structure_set/`),
/// by tag id, with the tags they include expanded. Optional entries
/// (`{"id": ..., "required": false}`) count as their id.
pub fn structure_set_tags<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> BTreeMap<String, Vec<String>> {
    let mut raw: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let Some(id) = entry
            .name()
            .strip_prefix("data/")
            .and_then(|r| r.split_once("/tags/worldgen/structure_set/"))
            .and_then(|(ns, path)| Some(format!("{ns}:{}", path.strip_suffix(".json")?)))
        else {
            continue;
        };
        let mut text = String::new();
        let Ok(v) = entry.read_to_string(&mut text).map_err(drop).and_then(|_| {
            serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).map_err(drop)
        }) else {
            continue;
        };
        let values = v["values"].as_array().into_iter().flatten();
        raw.insert(
            id,
            values
                .filter_map(|e| e.as_str().or_else(|| e["id"].as_str()).map(str::to_owned))
                .collect(),
        );
    }
    let expand = |tag: &str| {
        let (mut out, mut seen, mut todo) = (Vec::new(), BTreeSet::new(), vec![tag.to_owned()]);
        while let Some(t) = todo.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            for e in raw.get(&t).into_iter().flatten() {
                match e.strip_prefix('#') {
                    Some(inner) => todo.push(inner.to_owned()),
                    None if !out.contains(e) => out.push(e.clone()),
                    None => {}
                }
            }
        }
        out
    };
    raw.keys().map(|t| (t.clone(), expand(t))).collect()
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

    const NO_TAGS: BTreeMap<String, Vec<String>> = BTreeMap::new();

    #[test]
    fn maps_library_types_to_vanillas() {
        let mut report = Upgrade::default();
        let mut s = json!({"type": "lithostitched:jigsaw", "start_pool": "a:b"});
        assert!(upgrade("structure", &mut s, &mut report, &NO_TAGS));
        assert_eq!(s["type"], "minecraft:jigsaw");
        let mut nether =
            json!({"type": "moogs_structures:moogs_structures_generic_nether_jigsaw_structure"});
        assert!(!upgrade("structure", &mut nether, &mut report, &NO_TAGS));
        let mut set = json!({"placement": {"type": "moogs_structures:advanced_random_spread",
            "spacing": 10, "separation": 5, "min_distance_from_world_origin": 250}});
        assert!(upgrade("structure_set", &mut set, &mut report, &NO_TAGS));
        assert_eq!(set["placement"]["type"], "minecraft:random_spread");
        // Moog's spreads 1.65 times as far as its file says.
        assert_eq!(set["placement"]["spacing"], 17);
        assert_eq!(set["placement"]["separation"], 8);
        assert_eq!(
            set["placement"]["horizon:min_distance_from_world_origin"],
            250
        );
        assert!(
            set["placement"]
                .get("min_distance_from_world_origin")
                .is_none()
        );
        let mut pool = json!({"elements": [
            {"weight": 1, "element": {"element_type": "moogs_structures:versioned_single_pool_element", "location": "m:x"}},
            {"weight": 1, "element": {"element_type": "minecraft:list_pool_element", "elements": [
                {"element_type": "empty_pool_element"}]}},
            {"weight": 1, "element": {"element_type": "somelib:element"}}
        ]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        assert_eq!(
            pool["elements"][0]["element"]["element_type"],
            "minecraft:single_pool_element"
        );
        assert_eq!(
            pool["elements"][1]["element"]["elements"][0]["element_type"],
            "minecraft:empty_pool_element"
        );
        assert_eq!(report.mapped, 3);
        assert_eq!(report.block_states, 0);
        assert_eq!(
            report.unsupported,
            BTreeMap::from([
                (
                    "moogs_structures:moogs_structures_generic_nether_jigsaw_structure".into(),
                    1
                ),
                ("somelib:element".into(), 1),
            ])
        );
    }

    #[test]
    fn keeps_the_jigsaws_start_rules() {
        let mut report = Upgrade::default();
        let mut moogs = json!({"type": "moogs_structures:moogs_structures_generic_jigsaw_structure",
            "cannot_spawn_in_liquid": true, "terrain_height_radius_check": 2,
            "allowed_terrain_height_range": 12, "valid_biome_radius_check": 1,
            "y_allowance": {"min_y_allowed": 60}, "burying_type": "lowest_corner",
            "use_bounding_box_hack": false, "size": 7});
        assert!(upgrade("structure", &mut moogs, &mut report, &NO_TAGS));
        assert_eq!(
            moogs["horizon:start_checks"],
            json!({"liquid": "top", "terrain_radius": 2, "terrain_range": 12,
                "biome_radius": 1, "min_y": 60})
        );
        for gone in [
            "cannot_spawn_in_liquid",
            "y_allowance",
            "burying_type",
            "use_bounding_box_hack",
        ] {
            assert!(moogs.get(gone).is_none(), "{gone}");
        }
        assert_eq!(moogs["size"], 7);
        // Repurposed Structures reads the block above the bottom.
        let mut rs = json!({"type": "repurposed_structures:generic_jigsaw_structure",
            "cannot_spawn_in_liquid": true});
        assert!(upgrade("structure", &mut rs, &mut report, &NO_TAGS));
        assert_eq!(rs["horizon:start_checks"], json!({"liquid": "lowest"}));
        // Nothing to check: no key.
        let mut plain = json!({"type": "lithostitched:jigsaw", "cannot_spawn_in_liquid": false});
        assert!(upgrade("structure", &mut plain, &mut report, &NO_TAGS));
        assert!(plain.get("horizon:start_checks").is_none());
        assert_eq!(
            report.rules_lost,
            BTreeMap::from([("burying_type".into(), 1), ("y_allowance".into(), 1)])
        );
    }

    #[test]
    fn super_exclusion_zones_name_their_sets() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, text) in [
            (
                "data/rs/tags/worldgen/structure_set/avoid.json",
                r##"{"values": ["minecraft:villages", "#rs:more", {"id": "x:y", "required": false}]}"##,
            ),
            (
                "data/rs/tags/worldgen/structure_set/more.json",
                r##"{"values": ["minecraft:villages", "rs:cities", "#rs:avoid"]}"##,
            ),
        ] {
            zip.start_file(name, opts).unwrap();
            std::io::Write::write_all(&mut zip, text.as_bytes()).unwrap();
        }
        let mut zip = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        let tags = structure_set_tags(&mut zip);
        assert_eq!(tags["rs:avoid"], ["minecraft:villages", "x:y", "rs:cities"]);
        let mut report = Upgrade::default();
        let mut set = json!({"placement": {"type": "repurposed_structures:advanced_random_spread",
            "spacing": 30, "separation": 10,
            "super_exclusion_zone": {"chunk_count": 4, "other_set": "#rs:avoid"}}});
        assert!(upgrade("structure_set", &mut set, &mut report, &tags));
        assert_eq!(set["placement"]["spacing"], 30);
        assert_eq!(
            set["placement"]["horizon:exclusion_zone"],
            json!({"other_sets": ["minecraft:villages", "x:y", "rs:cities"], "chunk_count": 4})
        );
        let mut unknown = json!({"placement": {"type": "repurposed_structures:advanced_random_spread",
            "spacing": 30, "separation": 10,
            "super_exclusion_zone": {"chunk_count": 4, "other_set": "#rs:none"}}});
        assert!(upgrade("structure_set", &mut unknown, &mut report, &tags));
        assert_eq!(report.rules_lost["super_exclusion_zone"], 1);
    }

    #[test]
    fn yungs_jigsaws_keep_their_rules() {
        let mut report = Upgrade::default();
        let mut s = json!({"type": "yungsapi:yung_jigsaw", "start_pool": "y:start", "size": 20,
            "max_y": 60, "x_offset_in_chunk": 0, "max_distance_from_center": 80,
            "enhanced_terrain_adaptation": {"type": "yungsapi:custom", "kernel_size": 24}});
        assert!(upgrade("structure", &mut s, &mut report, &NO_TAGS));
        assert_eq!(s["type"], "minecraft:jigsaw");
        assert_eq!(s["horizon:yungsapi"], json!({"max_y": 60}));
        for gone in ["max_y", "x_offset_in_chunk", "enhanced_terrain_adaptation"] {
            assert!(s.get(gone).is_none(), "{gone}");
        }
        let mut pool = json!({"elements": [{"weight": 3, "element": {
            "element_type": "yungsapi:yung_single_element", "location": "y:room",
            "processors": "minecraft:empty", "projection": "rigid",
            "name": "room", "max_count": 2, "min_required_depth": 1, "is_priority": true,
            "ignore_bounds": false, "deadend_pool": "y:end",
            "condition": {"type": "yungsapi:all_of", "conditions": [
                {"type": "yungsapi:piece_in_horizontal_direction", "pieces": ["yungsapi:all"],
                    "range": 5, "rotation": "none"},
                {"type": "yungsapi:not", "condition": {"type": "yungsapi:mod_loaded", "modid": "create"}},
                {"type": "yungsapi:biome", "biomes": []}]}}}]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        let e = &pool["elements"][0]["element"];
        assert_eq!(e["element_type"], "minecraft:single_pool_element");
        assert_eq!(e["location"], "y:room");
        assert_eq!(
            e["horizon:yungsapi"],
            json!({"name": "room", "max_count": 2, "min_depth": 1, "priority": true,
                "ignore_bounds": false, "condition": {"type": "all_of", "conditions": [
                    {"type": "piece_in_horizontal_direction", "pieces": [], "range": 5, "rotation": "none"},
                    {"type": "not", "condition": {"type": "false"}},
                    {"type": "false"}]}})
        );
        assert!(e.get("deadend_pool").is_none());
        assert_eq!(
            report.rules_lost,
            BTreeMap::from([
                ("enhanced_terrain_adaptation".into(), 1),
                ("yungsapi condition yungsapi:biome".into(), 1),
                ("yungsapi deadend_pool".into(), 1),
            ])
        );
        assert!(report.unsupported.is_empty());
    }

    #[test]
    fn drops_processors_written_in_java() {
        let mut report = Upgrade::default();
        let mut list = json!({"processors": [
            {"processor_type": "repurposed_structures:bottom_pillar_processor"},
            {"processor_type": "minecraft:gravity", "heightmap": "WORLD_SURFACE_WG", "offset": -1},
            {"processor_type": "moogs_structures:spawner_randomizing_processor"}]});
        assert!(upgrade("processor_list", &mut list, &mut report, &NO_TAGS));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "minecraft:gravity",
            "heightmap": "WORLD_SURFACE_WG", "offset": -1}])
        );
        let mut pool = json!({"elements": [
            {"weight": 1, "element": {"element_type": "minecraft:single_pool_element",
                "processors": {"processors": [{"processor_type": "lithostitched:block_swap"}]}}},
            {"weight": 1, "element": {"element_type": "minecraft:list_pool_element", "elements": [
                {"element_type": "minecraft:single_pool_element", "processors": "x:named"}]}}]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        assert_eq!(
            pool["elements"][0]["element"]["processors"],
            json!({"processors": []})
        );
        assert_eq!(
            pool["elements"][1]["element"]["elements"][0]["processors"],
            "x:named"
        );
        let mut vanilla = json!({"processors": [{"processor_type": "minecraft:nop"}]});
        assert!(!upgrade(
            "processor_list",
            &mut vanilla,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(
            report.processors_dropped,
            BTreeMap::from([
                ("lithostitched:block_swap".into(), 1),
                ("moogs_structures:spawner_randomizing_processor".into(), 1),
                ("repurposed_structures:bottom_pillar_processor".into(), 1),
            ])
        );
    }

    #[test]
    fn yungs_placements_keep_their_rules() {
        let tags = BTreeMap::from([(
            "bf:avoid".to_owned(),
            vec!["minecraft:nether_complexes".to_owned()],
        )]);
        let mut report = Upgrade::default();
        let mut fortress = json!({"placement": {"type": "yungsapi:enhanced_random_spread",
            "salt": 1, "spacing": 30, "separation": 20,
            "enhanced_exclusion_zone": {"other_set": "#bf:avoid", "chunk_count": 10}}});
        assert!(upgrade("structure_set", &mut fortress, &mut report, &tags));
        assert_eq!(
            fortress["placement"],
            json!({"type": "minecraft:random_spread", "salt": 1, "spacing": 30, "separation": 20,
                "horizon:exclusion_zone": {"other_sets": ["minecraft:nether_complexes"], "chunk_count": 10}})
        );
        let mut temple = json!({"placement": {"type": "betterdeserttemples:desert_temple",
            "salt": 2, "spacing": 30, "separation": 20,
            "exclusion_zone": {"other_set": "minecraft:villages", "chunk_count": 6}}});
        assert!(upgrade("structure_set", &mut temple, &mut report, &tags));
        assert_eq!(temple["placement"]["type"], "minecraft:random_spread");
        assert_eq!(temple["placement"]["exclusion_zone"]["chunk_count"], 6);
        assert_eq!(
            temple["placement"]["horizon:avoid_biomes"],
            json!({"biomes": ["#minecraft:is_river", "#minecraft:is_ocean"],
                "radius": 48, "step": 2, "y": 120})
        );
        let mut stronghold = json!({"placement": {"type": "betterstrongholds:stronghold",
            "salt": 3, "spacing": 85, "separation": 50,
            "chunk_distance_to_first_ring": 80, "ring_chunk_thickness": 96}});
        assert!(upgrade(
            "structure_set",
            &mut stronghold,
            &mut report,
            &tags
        ));
        assert_eq!(
            stronghold["placement"],
            json!({"type": "minecraft:random_spread", "salt": 3, "spacing": 85, "separation": 50,
                "horizon:ring_sections": {"distance_to_first_ring": 80, "thickness": 96}})
        );
        assert_eq!(report.mapped, 3);
        assert!(report.unsupported.is_empty());
        // The vanilla counterpart is turned off only for a structure the game starts.
        let mut ported = json!({"type": "minecraft:jigsaw"});
        assert!(disables("betterstrongholds:stronghold", &mut ported));
        assert_eq!(ported["horizon:disables"], json!(["minecraft:stronghold"]));
        let mut java = json!({"type": "bettermineshafts:mineshaft"});
        assert!(!disables("bettermineshafts:mineshaft_oak", &mut java));
        let mut circle = json!({"type": "minecraft:jigsaw"});
        assert!(!disables("betterwitchhuts:witch_circle", &mut circle));
    }

    #[test]
    fn writes_block_states_in_26_3_shape() {
        let mut report = Upgrade::default();
        let mut list = json!({"processors": [{"processor_type": "minecraft:rule", "rules": [{
            "input_predicate": {"predicate_type": "minecraft:always_true"},
            "output_state": {"Name": "minecraft:oak_stairs", "Properties": {"facing": "east"}}}]}]});
        assert!(upgrade("processor_list", &mut list, &mut report, &NO_TAGS));
        assert_eq!(
            list["processors"][0]["rules"][0]["output_state"],
            json!({"id": "minecraft:oak_stairs", "properties": {"facing": "east"}})
        );
        let mut spring = json!({"type": "minecraft:spring_feature", "config": {
            "state": {"Name": "minecraft:water"}, "valid_blocks": ["minecraft:stone"]}});
        assert!(upgrade(
            "configured_feature",
            &mut spring,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(spring["config"]["state"], json!({"id": "minecraft:water"}));
        // An object with other keys is not a state; a 26.3 state is left as it is.
        let mut other = json!({"Name": "x", "weight": 1, "state": {"id": "minecraft:stone"}});
        assert!(!upgrade(
            "configured_feature",
            &mut other,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(report.block_states, 2);
        assert_eq!(report.mapped, 0);
    }

    #[test]
    fn modifiers_add_to_sets_and_pools() {
        let mut report = Upgrade::default();
        let only_fd = |m: &str| m == "farmersdelight";
        let set = json!({"type": "lithostitched:add_structure_set_entries",
            "structure_sets": "minecraft:villages",
            "entries": [{"structure": "x:village", "weight": 4}]});
        let (kind, added) = modifier(&set, &only_fd, &mut report).unwrap();
        assert_eq!(kind, "structure_set");
        assert_eq!(
            added,
            json!({"horizon:add_to": ["minecraft:villages"],
                "structures": [{"structure": "x:village", "weight": 4}]})
        );
        // A compat addition holds only when its mod is ported along.
        let pool = |modid: &str| {
            json!({"type": "lithostitched:add_template_pool_elements",
                "neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": modid}],
                "fabric:load_conditions": {"condition": "fabric:all_mods_loaded", "values": ["other"]},
                "template_pools": ["x:houses", "y:houses"],
                "elements": [{"weight": 5, "element": {"element_type": "minecraft:single_pool_element",
                    "location": "x:fd_farm", "projection": "rigid", "processors": "minecraft:empty"}}]})
        };
        let (kind, added) = modifier(&pool("farmersdelight"), &only_fd, &mut report).unwrap();
        assert_eq!(kind, "template_pool");
        assert_eq!(added["horizon:add_to"], json!(["x:houses", "y:houses"]));
        assert_eq!(added["elements"][0]["weight"], 5);
        assert!(modifier(&pool("create"), &only_fd, &mut report).is_none());
        // Fabric's conditions when there are no NeoForge ones; lithostitched's own.
        let fabric = json!({"type": "lithostitched:add_structure_set_entries",
            "fabric:load_conditions": [{"condition": "fabric:not",
                "value": {"condition": "fabric:any_mods_loaded", "values": ["create"]}}],
            "predicate": {"type": "lithostitched:all_of", "predicates": [
                {"type": "lithostitched:mod_loaded", "mod_id": "farmersdelight"},
                {"type": "lithostitched:true"}]},
            "structure_sets": "minecraft:villages", "entries": []});
        assert!(modifier(&fabric, &only_fd, &mut report).is_some());
        let tagged = json!({"type": "lithostitched:add_structure_set_entries",
            "structure_sets": "#x:sets", "entries": []});
        assert!(modifier(&tagged, &only_fd, &mut report).is_none());
        let features = json!({"type": "lithostitched:add_features"});
        assert!(modifier(&features, &only_fd, &mut report).is_none());
        assert_eq!(
            report.added_to,
            BTreeMap::from([
                ("minecraft:villages".into(), 1),
                ("x:houses".into(), 1),
                ("y:houses".into(), 1),
            ])
        );
        assert_eq!(
            report.modifiers_left,
            BTreeMap::from([
                ("conditions".into(), 1),
                ("lithostitched:add_features".into(), 1),
                ("target".into(), 1),
            ])
        );
    }

    #[test]
    fn ctov_joins_vanillas_villages_and_outposts() {
        let sets = code_set_entries("ctov");
        let (file, set, outposts) = &sets[0];
        assert_eq!(
            (*file, *set),
            ("pillager_outposts", "minecraft:pillager_outposts")
        );
        assert_eq!(outposts.len(), 11);
        assert!(outposts.iter().all(|(_, w)| *w == 1));
        let (_, set, villages) = &sets[1];
        assert_eq!(*set, "minecraft:villages");
        assert_eq!(villages.len(), 63);
        assert_eq!(villages[0], ("ctov:small/village_beach".into(), 10));
        assert_eq!(villages[21], ("ctov:medium/village_beach".into(), 4));
        assert_eq!(
            villages[62],
            ("ctov:large/village_taiga_fortified".into(), 1)
        );
        assert!(code_set_entries("mvs").is_empty());
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
