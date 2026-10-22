//! Structure processors the structure libraries write in Java, carried over
//! as data the game runs: the ones that give spawners their mob.
//!
//! - Moog's Structure Lib's and Repurposed Structures' spawner randomizing
//!   processors pick a mob from a weighted list for each spawner. They become
//!   `horizon:spawner_mobs` (`horizon_world::worldgen::structure::processor`),
//!   with Repurposed's `rs_spawners` list written in.
//! - YUNG's Better Dungeons and Better Desert Temples turn a spawner or a
//!   placeholder block into a spawner of a fixed mob. That is vanilla's `rule`
//!   processor with an `append_static` block entity.
//! - Moog's vanilla loot swap only acts when its mod's "replace vanilla"
//!   preset is on; one that ships off is a no-op and is taken out.
//! - Lithostitched's `reference` runs other processor lists in turn: their
//!   processors are written in its place (Choice Theorem's Overhauled
//!   Villages builds most of its lists so). Its `apply_random` and
//!   `condition` become the game's `horizon:apply_random` and
//!   `horizon:condition`, with the lists inside them carried the same way,
//!   and its `set_block` (simple or weighted states) `horizon:set_block`.
//! - Repurposed Structures' mineshaft sky view becomes the game's
//!   `horizon:mineshaft_sky_view`.
//! - Choice Theorem's Overhauled Village's `modular_compat` runs another
//!   list only when the mod it names is loaded: that list is written in its
//!   place when the mod is ported along, and it is taken out (a no-op, as in
//!   the mod) when it is not.
//!
//! What is not here is taken out of its list later and counted
//! (`worldgen::upgrade`). The entity NBT is written in 26.3's shape (an
//! `equipment` map, not 1.21.1's `HandItems` and `ArmorItems`).
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek},
};

/// What the processors read from the mod's own data files.
#[derive(Debug, Default)]
pub struct ProcessorData {
    /// Repurposed Structures' `rs_spawners/<path>.json` by id: each mob and
    /// its weight, without the weight-0 entries its loader removes.
    pub spawners: BTreeMap<String, Vec<(String, f64)>>,
    /// Moog's "replace vanilla" presets that are on by default:
    /// `(mod id, vanilla_key)`.
    pub replace_vanilla: BTreeSet<(String, String)>,
    /// The mod's processor lists by id, for Lithostitched's references.
    pub lists: BTreeMap<String, Value>,
    /// The mods named by its `ctov:modular_compat` processors that are
    /// ported along ([`Self::mark_loaded`]).
    pub compat_loaded: BTreeSet<String>,
}

/// Choice Theorem's Overhauled Village's processor that runs the list
/// `reference` when mod `modid` is loaded (ModularCompatProcessor).
const MODULAR_COMPAT: &str = "ctov:modular_compat";

impl ProcessorData {
    /// Notes which mods its compat processors name are `loaded`.
    pub fn mark_loaded(&mut self, loaded: &dyn Fn(&str) -> bool) {
        fn walk(v: &Value, out: &mut BTreeSet<String>) {
            match v {
                Value::Object(o) => {
                    if o.get("processor_type").and_then(Value::as_str) == Some(MODULAR_COMPAT) {
                        if let Some(id) = o.get("modid").and_then(Value::as_str) {
                            out.insert(id.to_owned());
                        }
                    }
                    o.values().for_each(|v| walk(v, out));
                }
                Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
                _ => {}
            }
        }
        let mut named = BTreeSet::new();
        self.lists.values().for_each(|l| walk(l, &mut named));
        self.compat_loaded = named.into_iter().filter(|m| loaded(m)).collect();
    }

    pub fn read<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Self {
        let mut out = Self::default();
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else {
                continue;
            };
            let name = entry.name().to_owned();
            let Some((ns, rest)) = name
                .strip_prefix("data/")
                .and_then(|r| r.strip_suffix(".json"))
                .and_then(|r| r.split_once('/'))
            else {
                continue;
            };
            let spawners = rest.strip_prefix("rs_spawners/");
            let list = rest.strip_prefix("worldgen/processor_list/");
            if spawners.is_none() && list.is_none() && rest != "moogs_structures/replace_vanilla" {
                continue;
            }
            let mut text = String::new();
            let Ok(v) = entry
                .read_to_string(&mut text)
                .map_err(|_| ())
                .and_then(|_| serde_json::from_str::<Value>(&text).map_err(|_| ()))
            else {
                continue;
            };
            if let Some(path) = list {
                out.lists.insert(format!("{ns}:{path}"), v);
                continue;
            }
            if let Some(path) = spawners {
                let mobs = v["mobs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| Some((m["name"].as_str()?.to_owned(), m["weight"].as_f64()?)))
                    .filter(|(_, w)| *w != 0.)
                    .collect();
                out.spawners.insert(format!("{ns}:{path}"), mobs);
                continue;
            }
            for preset in v["presets"].as_array().into_iter().flatten() {
                if preset["default_enabled"].as_bool() != Some(true) {
                    continue;
                }
                for r in preset["replacements"].as_array().into_iter().flatten() {
                    if let Some(key) = r["vanilla_key"].as_str() {
                        out.replace_vanilla.insert((ns.to_owned(), key.to_owned()));
                    }
                }
            }
        }
        out
    }
}

/// Calls `f` on each processor list in a worldgen file: a `processor_list`
/// file, and the inline lists of a template pool's elements (list elements
/// included). Whether any call changed something.
pub fn each_list(kind: &str, v: &mut Value, f: &mut dyn FnMut(&mut Value) -> bool) -> bool {
    match kind {
        "processor_list" => f(v),
        "template_pool" => {
            let mut changed = false;
            for e in v["elements"].as_array_mut().into_iter().flatten() {
                let element = &mut e["element"];
                if let Some(list) = element.get_mut("elements").and_then(Value::as_array_mut) {
                    for inner in list {
                        if let Some(p) = inner.get_mut("processors") {
                            changed |= f(p);
                        }
                    }
                } else if let Some(p) = element.get_mut("processors") {
                    changed |= f(p);
                }
            }
            changed
        }
        _ => false,
    }
}

/// The processors of a list: `{"processors": [...]}` or a bare list (an id
/// names another file and is left alone).
pub fn processors(list: &mut Value) -> Option<&mut Vec<Value>> {
    match list {
        Value::Array(a) => Some(a),
        Value::Object(o) => o.get_mut("processors").and_then(Value::as_array_mut),
        _ => None,
    }
}

/// Rewrites the processors of a worldgen file that have a data form; each is
/// counted by type in `carried`. Whether anything changed.
pub fn carry(
    kind: &str,
    v: &mut Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> bool {
    each_list(kind, v, &mut |list| {
        let Some(list) = processors(list) else {
            return false;
        };
        let (out, changed) = carry_all(std::mem::take(list), data, carried);
        *list = out;
        changed
    })
}

/// The processors of one list carried: references written in, then each
/// one with a data form replaced by it. Whether anything changed.
fn carry_all(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    carry_at(list, data, carried, 0)
}

/// [`carry_all`] for a list `depth` pillars deep (see [`pillar`]).
fn carry_at(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
    depth: usize,
) -> (Vec<Value>, bool) {
    let (expanded, mut changed) = references(list, data, carried, 0);
    let mut out = Vec::with_capacity(expanded.len());
    for p in expanded {
        let kind = p["processor_type"].as_str().unwrap_or_default().to_owned();
        let replacement = match kind.as_str() {
            "lithostitched:apply_random" => apply_random(&p, data, carried).map(|p| vec![p]),
            "lithostitched:condition" => condition(&p, data, carried).map(|p| vec![p]),
            _ => match pillar_type(&kind) {
                Some(to) => Some(vec![pillar(&p, to, data, carried, depth)]),
                None => as_data(&kind, &p, data),
            },
        };
        match replacement {
            Some(replacement) => {
                *carried.entry(kind).or_default() += 1;
                out.extend(replacement);
                changed = true;
            }
            None => out.push(p),
        }
    }
    (out, changed)
}

/// The game's type for the mods' processors that place blocks past their
/// template, which keep their fields (`horizon-world`'s `pillars.rs`).
fn pillar_type(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "repurposed_structures:pillar_processor" | "moogs_structures:pillar_processor" => {
            "horizon:pillar"
        }
        "repurposed_structures:bottom_pillar_processor" => "horizon:bottom_pillar",
        "repurposed_structures:close_off_fluid_sources_processor"
        | "moogs_structures:close_off_fluid_sources_processor" => "horizon:close_off_fluid_sources",
        "repurposed_structures:close_off_air_sources_processor" => "horizon:close_off_air_sources",
        "betterfortresses:pillar_processor" | "betterjungletemples:pillar_processor" => {
            "horizon:yung_pillar"
        }
        "betterfortresses:stair_pillar_processor" => "horizon:stair_pillar",
        _ => return None,
    })
}

/// A pillar processor as the game's `to`. Its `pillar_processor_list`
/// (which runs on each block of the column; Repurposed's often names the
/// list it is in) is written in, carried; the pillars inside that copy lose
/// theirs, as a column's blocks are never the mods' triggers. Better
/// Fortresses' stair supports get the blocks its Java code names.
fn pillar(
    p: &Value,
    to: &str,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
    depth: usize,
) -> Value {
    if to == "horizon:stair_pillar" {
        return json!({"processor_type": to,
            "blocks": {"minecraft:stone_brick_stairs": "minecraft:red_nether_brick_stairs",
                "minecraft:purpur_stairs": "minecraft:red_nether_bricks"},
            "front": "minecraft:red_nether_brick_stairs",
            "pillar": "minecraft:red_nether_bricks"});
    }
    let mut out = p.clone();
    out["processor_type"] = to.into();
    let Some(o) = out.as_object_mut() else {
        return out;
    };
    let Some(list) = o.remove("pillar_processor_list") else {
        return out;
    };
    let inner = match &list {
        Value::String(id) if id == "minecraft:empty" => None,
        _ if depth > 0 => None,
        other => referenced(other, data),
    };
    if let Some(inner) = inner {
        let (inner, _) = carry_at(inner, data, carried, depth + 1);
        o.insert("pillar_processor_list".into(), json!({"processors": inner}));
    }
    out
}

/// A processor list inside a carried processor: an id stays (the game reads
/// that file, carried on its own); an inline one is carried here.
fn nested(list: &Value, data: &ProcessorData, carried: &mut BTreeMap<String, usize>) -> Value {
    let mut list = list.clone();
    if let Some(ps) = processors(&mut list) {
        *ps = carry_all(std::mem::take(ps), data, carried).0;
    }
    list
}

/// Lithostitched's `apply_random` as `horizon:apply_random`: its weighted
/// lists, or its set of lists at weight 1. A tag is not carried.
fn apply_random(
    p: &Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> Option<Value> {
    let lists = &p["processor_lists"];
    let weighted = lists.as_array().is_some_and(|a| {
        a.iter()
            .all(|e| e.get("weight").is_some() && e.get("data").is_some())
    });
    let entries: Vec<(Value, Value)> = if weighted {
        lists
            .as_array()?
            .iter()
            .map(|e| (e["weight"].clone(), e["data"].clone()))
            .collect()
    } else {
        match lists {
            Value::String(id) if !id.starts_with('#') => vec![(1.into(), lists.clone())],
            Value::Array(a) if a.iter().any(|x| x.get("processor_type").is_some()) => {
                vec![(1.into(), lists.clone())]
            }
            Value::Array(a)
                if a.iter()
                    .all(|x| x.as_str().is_some_and(|s| !s.starts_with('#')) || x.is_object()) =>
            {
                a.iter().map(|x| (1.into(), x.clone())).collect()
            }
            Value::Object(_) => vec![(1.into(), lists.clone())],
            _ => return None,
        }
    };
    let lists: Vec<Value> = entries
        .into_iter()
        .map(|(weight, list)| json!({"weight": weight, "data": nested(&list, data, carried)}))
        .collect();
    let mut out = json!({"processor_type": "horizon:apply_random", "processor_lists": lists});
    if let Some(mode) = p.get("mode") {
        out["mode"] = mode.clone();
    }
    Some(out)
}

/// Lithostitched's `condition` as `horizon:condition`, when the game runs
/// every condition in it (true, all_of, any_of, not, random_chance,
/// matching_blocks with plain property values); else it stays Java.
fn condition(
    p: &Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> Option<Value> {
    fn runs(c: &Value) -> bool {
        if let Some(all) = c.as_array() {
            return all.iter().all(runs);
        }
        let kind = c["type"].as_str().unwrap_or_default();
        match kind.strip_prefix("lithostitched:").unwrap_or(kind) {
            "true" | "random_chance" => true,
            "all_of" | "any_of" => c["conditions"]
                .as_array()
                .is_some_and(|a| a.iter().all(runs)),
            "not" => runs(&c["condition"]),
            "matching_blocks" => c.get("properties").is_none_or(|p| {
                p.as_object()
                    .is_some_and(|o| o.values().all(Value::is_string))
            }),
            _ => false,
        }
    }
    if !runs(&p["if_true"]) {
        return None;
    }
    // One processor or a list of them.
    let mut branch = |key: &str| -> Value {
        let list = match p.get(key) {
            None => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(one) => vec![one.clone()],
        };
        carry_all(list, data, carried).0.into()
    };
    let (then, otherwise) = (branch("then"), branch("else"));
    let mut out = json!({"processor_type": "horizon:condition", "if_true": p["if_true"],
        "then": then, "else": otherwise});
    if let Some(mode) = p.get("random_mode") {
        out["random_mode"] = mode.clone();
    }
    Some(out)
}

/// Repurposed Structures' rule tests and the game's types that run them
/// (`horizon_world::worldgen`'s `rule_test` and `structure::processor`).
const RULE_TESTS: [(&str, &str); 3] = [
    (
        "repurposed_structures:matter_phase_rule_test",
        "horizon:matter_phase",
    ),
    (
        "repurposed_structures:piece_origin_axis_aligned_linear_pos_rule_test",
        "horizon:piece_origin_axis_aligned_linear_pos",
    ),
    (
        "repurposed_structures:y_value_pos_rule_test",
        "horizon:y_value",
    ),
];

/// Renames the rule tests written in Java that the game runs under its own
/// type ([`RULE_TESTS`]) in a worldgen file's processor lists; each is
/// counted by type in `carried`. Their fields are the mod's. Whether
/// anything changed. Without this the game, like vanilla, refuses the list
/// and every pool naming it: Repurposed's mineshafts and Nether monument.
pub fn rule_tests(kind: &str, v: &mut Value, carried: &mut BTreeMap<String, usize>) -> bool {
    fn rename(v: &mut Value, carried: &mut BTreeMap<String, usize>) -> bool {
        match v {
            Value::Object(o) => {
                let mut changed = false;
                if let Some(Value::String(t)) = o.get_mut("predicate_type")
                    && let Some((old, new)) = RULE_TESTS.iter().find(|(old, _)| t == old)
                {
                    *carried.entry((*old).to_owned()).or_default() += 1;
                    *t = (*new).to_owned();
                    changed = true;
                }
                o.values_mut().fold(changed, |c, x| rename(x, carried) | c)
            }
            Value::Array(a) => a.iter_mut().fold(false, |c, x| rename(x, carried) | c),
            _ => false,
        }
    }
    each_list(kind, v, &mut |list| rename(list, carried))
}

/// Lithostitched's references in a list, each replaced by the processors of
/// the lists it names, in order (references in them too, to a depth of 8).
/// One naming a tag or a list the mod does not have stays, to be dropped.
/// Whether any was replaced.
fn references(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
    depth: usize,
) -> (Vec<Value>, bool) {
    let mut out = Vec::with_capacity(list.len());
    let mut changed = false;
    for p in list {
        if p["processor_type"] == MODULAR_COMPAT && depth < 8 {
            *carried.entry(MODULAR_COMPAT.into()).or_default() += 1;
            changed = true;
            let loaded = p["modid"]
                .as_str()
                .is_some_and(|m| data.compat_loaded.contains(m));
            if let Some(inner) = loaded.then(|| referenced(&p["reference"], data)).flatten() {
                out.extend(references(inner, data, carried, depth + 1).0);
            }
            continue;
        }
        let named = (p["processor_type"] == "lithostitched:reference" && depth < 8)
            .then(|| referenced(&p["processor_lists"], data))
            .flatten();
        match named {
            Some(inner) => {
                *carried.entry("lithostitched:reference".into()).or_default() += 1;
                out.extend(references(inner, data, carried, depth + 1).0);
                changed = true;
            }
            None => out.push(p),
        }
    }
    (out, changed)
}

/// The processors of the lists a reference names: an id, an inline list, or
/// a list of either.
fn referenced(lists: &Value, data: &ProcessorData) -> Option<Vec<Value>> {
    let one = |l: &Value| -> Option<Vec<Value>> {
        let mut list = match l {
            Value::String(id) if !id.starts_with('#') => {
                let id = if id.contains(':') {
                    id.clone()
                } else {
                    format!("minecraft:{id}")
                };
                data.lists.get(&id)?.clone()
            }
            Value::Object(_) | Value::Array(_) => l.clone(),
            _ => return None,
        };
        processors(&mut list).map(std::mem::take)
    };
    match lists {
        // A bare list of processors is one inline list; else a list of lists.
        Value::Array(a) if a.iter().any(|x| x.get("processor_type").is_some()) => Some(a.clone()),
        Value::Array(a) => Some(a.iter().map(one).collect::<Option<Vec<_>>>()?.concat()),
        other => one(other),
    }
}

/// The data form of a Java processor: the processors to put in its place
/// (none for a no-op), or `None` when it has none.
fn as_data(kind: &str, p: &Value, data: &ProcessorData) -> Option<Vec<Value>> {
    let (ns, _) = kind.split_once(':')?;
    Some(match kind {
        // Its structure voids keep the world's block, as vanilla's
        // block_ignore does (the templates hold them; vanilla places them).
        "repurposed_structures:structure_void_processor" => vec![json!({
            "processor_type": "minecraft:block_ignore",
            "blocks": ["minecraft:structure_void"]})],
        "moogs_structures:spawner_randomizing_processor" => {
            let mobs = p["weighted_entities"]
                .as_array()?
                .iter()
                .map(|e| {
                    // The entry's NBT, then its id over it.
                    let mut entity = match &e["nbt"] {
                        Value::Object(o) => o.clone(),
                        _ => serde_json::Map::new(),
                    };
                    entity.insert("id".into(), e["entity"].as_str()?.into());
                    Some(json!({"entity": entity, "weight": e["weight"].as_f64()?}))
                })
                .collect::<Option<Vec<_>>>()?;
            vec![spawner_mobs(p, mobs, None)]
        }
        "repurposed_structures:spawner_randomizing_processor" => {
            // A list the mod does not ship falls back to a vanilla dungeon
            // mob in the mod; left out here.
            let list = data
                .spawners
                .get(p["rs_spawner_resourcelocation"].as_str()?)?;
            let mobs = list
                .iter()
                .map(|(id, weight)| json!({"entity": {"id": id}, "weight": weight}))
                .collect();
            vec![spawner_mobs(p, mobs, repurposed_light(p)?)]
        }
        // Wet where the world holds water, dry where it does not.
        "repurposed_structures:waterlogging_when_replacing_water_processor" => {
            vec![json!({"processor_type": "horizon:waterlog_in_world_water"})]
        }
        // Its mineshafts: nothing above the ocean floor but grounded rails.
        "repurposed_structures:mineshaft_sky_view_processor" => {
            vec![json!({"processor_type": "horizon:mineshaft_sky_view"})]
        }
        "moogs_structures:vanilla_loot_swap_processor" => {
            let key = (
                p["modid"].as_str()?.to_owned(),
                p["vanilla_key"].as_str()?.to_owned(),
            );
            if data.replace_vanilla.contains(&key) {
                return None;
            }
            Vec::new()
        }
        "moogs_structures:trial_spawner_randomizing_processor" => {
            // The configs by id: 26.3 reads them from `trial_spawner/`, the
            // folder Moog's loads them from.
            let mut nbt = json!({"normal_config": p["normal_config"].as_str()?});
            if let Some(ominous) = p["ominous_config"].as_str() {
                nbt["ominous_config"] = ominous.into();
            }
            // The state the template was saved with goes.
            let saved = [
                "server_data",
                "shared_data",
                "spawn_data",
                "cooldown_end_at_tick",
                "next_mob_spawns_at",
            ];
            vec![same_state_rule(&TRIAL_SPAWNER_STATES, &saved, |_| {
                nbt.clone()
            })]
        }
        "moogs_structures:vault_randomizing_processor" => {
            let normal = p["loot_table"].as_str()?;
            let config = |ominous: bool| {
                let (table, key) = match ominous {
                    true => (
                        p["ominous_loot_table"].as_str().unwrap_or(normal),
                        p["ominous_key_item"]
                            .as_str()
                            .unwrap_or("minecraft:ominous_trial_key"),
                    ),
                    false => (
                        normal,
                        p["key_item"].as_str().unwrap_or("minecraft:trial_key"),
                    ),
                };
                // Merged into the vault's config, as the mod puts both in.
                json!({"config": {"loot_table": table, "key_item": {"id": key, "count": 1}}})
            };
            vec![same_state_rule(
                &VAULT_STATES,
                &["server_data", "shared_data"],
                |s| config(s.get("ominous") == Some(&"true")),
            )]
        }
        _ if ns == "betterdeserttemples"
            && let Some(processor) = desert_temple(kind) =>
        {
            vec![processor]
        }
        "lithostitched:block_swap" => {
            // The same map; the game's swap keeps the properties both have.
            p["blocks"].as_object()?;
            vec![json!({"processor_type": "horizon:block_swap", "blocks": p["blocks"]})]
        }
        "lithostitched:set_block" => {
            // A simple or weighted state provider, the block entity left as
            // it is.
            let provider = p["state_provider"]["type"].as_str()?;
            let modifier = p["block_entity_modifier"]["type"].as_str();
            let plain =
                modifier.is_none_or(|m| m.trim_start_matches("minecraft:") == "passthrough");
            if !plain
                || !matches!(
                    provider,
                    "minecraft:simple_state_provider" | "minecraft:weighted_state_provider"
                )
            {
                return None;
            }
            let mut out = p.clone();
            out["processor_type"] = "horizon:set_block".into();
            if let Some(o) = out.as_object_mut() {
                o.remove("block_entity_modifier");
            }
            vec![out]
        }
        "betterdungeons:small_dungeon_chest_processor" => {
            // Its config's defaults: one chest at least, two at most, a
            // fifth of the others kept until then; ChestBlock covers both.
            vec![json!({"processor_type": "horizon:count_cap",
                "blocks": ["minecraft:chest", "minecraft:trapped_chest"],
                "min": 1, "max": 2, "chance": 0.2f32 as f64,
                "replacement": block("minecraft:cave_air")})]
        }
        "betterwitchhuts:leg_processor" => vec![yung_leg(
            "minecraft:brown_stained_glass",
            json!({"id": "minecraft:oak_log", "properties": {"axis": "y"}}),
            false,
        )],
        "betterwitchhuts:fence_leg_processor" => vec![yung_leg(
            "minecraft:crimson_fence",
            block("minecraft:oak_fence"),
            true,
        )],
        _ if ns == "betteroceanmonuments" => ocean_monument(kind)?,
        _ if ns == "betterdungeons" || ns == "betterdeserttemples" => vec![yung_spawner(kind, p)?],
        _ => return None,
    })
}

/// YUNG's leg processors (Better Ocean Monuments, Better Witch Huts): a
/// placeholder block becomes `state` and a column of it runs down through
/// air and fluids: the game's `horizon:yung_pillar` with fixed states, and
/// the placeholder's properties kept for the fence legs.
fn yung_leg(target: &str, state: Value, keep_properties: bool) -> Value {
    let fixed = json!({"entries": [], "defaultBlockState": state});
    json!({"processor_type": "horizon:yung_pillar", "target_block": block(target),
        "target_block_output": fixed, "pillar_states": fixed,
        "keep_properties": keep_properties})
}

/// Better Ocean Monuments' processors (no fields; what they do is in their
/// Java code): placeholder blocks as vanilla's `rule` or the game's
/// `horizon:pick` (a new `nextFloat` for each chance, as the mod draws),
/// and its sea-level ones as the game's `horizon:flood_air`,
/// `horizon:waterlog` and `horizon:keep_world_block`.
fn ocean_monument(kind: &str) -> Option<Vec<Value>> {
    let rule = |rules: &[(&str, Value)]| {
        let rules: Vec<Value> = rules
            .iter()
            .map(|(from, to)| {
                json!({"input_predicate": {"predicate_type": "minecraft:block_match", "block": from},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": to})
            })
            .collect();
        json!({"processor_type": "minecraft:rule", "rules": rules})
    };
    let state = |id: &str, props: Value| json!({"id": id, "properties": props});
    let pick = |from: &str, picks: Vec<(f32, Value)>, default: Value, keep: bool| {
        let picks: Vec<Value> = picks
            .into_iter()
            .map(|(below, state)| json!({"below": below as f64, "state": state}))
            .collect();
        json!({"processor_type": "horizon:pick", "block": from, "picks": picks,
            "default": {"state": default}, "redraw": true, "keep_properties": keep})
    };
    let slabs = |from: &str, slab: &str, otherwise: &str| {
        let wet = |t: &str| state(slab, json!({"type": t, "waterlogged": "true"}));
        pick(
            from,
            vec![(0.4, wet("top")), (0.8, wet("bottom"))],
            block(otherwise),
            false,
        )
    };
    // Exposed below 0.1, else weathered below a new 0.3, else oxidized,
    // with the placeholder's properties for the stairs and slabs.
    let copper = |kind: &str| {
        let id = |age: &str| block(&format!("minecraft:{age}{kind}"));
        pick(
            &format!("minecraft:oxidized_{kind}"),
            vec![(0.1, id("exposed_")), (0.3, id("weathered_"))],
            id("oxidized_"),
            true,
        )
    };
    Some(match kind.strip_prefix("betteroceanmonuments:")? {
        "random_prismarine_slab_decoration_processor" => vec![slabs(
            "minecraft:lime_concrete",
            "minecraft:prismarine_slab",
            "minecraft:air",
        )],
        "random_dark_prismarine_slab_decoration_processor" => vec![slabs(
            "minecraft:blue_concrete",
            "minecraft:dark_prismarine_slab",
            "minecraft:water",
        )],
        "sand_gravel_processor" => {
            vec![rule(&[(
                "minecraft:yellow_stained_glass",
                block("minecraft:gravel"),
            )])]
        }
        "random_oxidization_processor" => [
            "copper",
            "cut_copper",
            "cut_copper_stairs",
            "cut_copper_slab",
        ]
        .into_iter()
        .map(copper)
        .collect(),
        "structure_void_processor" => vec![json!({"processor_type": "horizon:keep_world_block",
            "blocks": ["minecraft:purpur_slab", "minecraft:brown_wool"]})],
        "air_processor" => vec![json!({"processor_type": "horizon:flood_air"})],
        "waterlog_processor" => vec![json!({"processor_type": "horizon:waterlog"})],
        "seagrass_processor" => vec![rule(&[
            (
                "minecraft:red_sandstone_slab",
                state("minecraft:tall_seagrass", json!({"half": "lower"})),
            ),
            (
                "minecraft:end_stone_brick_slab",
                state("minecraft:tall_seagrass", json!({"half": "upper"})),
            ),
            ("minecraft:brick_slab", block("minecraft:seagrass")),
        ])],
        "random_sponge_processor" => vec![pick(
            "minecraft:orange_stained_glass",
            vec![(0.75, block("minecraft:wet_sponge"))],
            block("minecraft:water"),
            false,
        )],
        "leg_processor" => vec![yung_leg(
            "minecraft:blue_stained_glass",
            block("minecraft:prismarine_bricks"),
            false,
        )],
        _ => return None,
    })
}

/// A block's states, as (block, [(property, values)]).
type States = (
    &'static str,
    &'static [(&'static str, &'static [&'static str])],
);
const TRIAL_SPAWNER_STATES: States = (
    "minecraft:trial_spawner",
    &[
        (
            "trial_spawner_state",
            &[
                "inactive",
                "waiting_for_players",
                "active",
                "waiting_for_reward_ejection",
                "ejecting_reward",
                "cooldown",
            ],
        ),
        ("ominous", &["false", "true"]),
    ],
);
const VAULT_STATES: States = (
    "minecraft:vault",
    &[
        ("facing", &["north", "south", "west", "east"]),
        (
            "vault_state",
            &["inactive", "active", "unlocking", "ejecting"],
        ),
        ("ominous", &["false", "true"]),
    ],
);

/// Moog's trial spawner and vault processors keep the block, take keys out
/// of its block entity and set others. As vanilla's `rule`: one rule per
/// state of the block, matching it and writing it back, with the game's
/// `horizon:edit` modifier taking out `remove` and merging `nbt(state)` in.
fn same_state_rule(
    states: &States,
    remove: &[&str],
    nbt: impl Fn(&BTreeMap<&str, &str>) -> Value,
) -> Value {
    let (block, properties) = states;
    let mut all = vec![BTreeMap::new()];
    for (key, values) in properties.iter() {
        all = all
            .into_iter()
            .flat_map(|s| {
                values.iter().map(move |v| {
                    let mut s = s.clone();
                    s.insert(*key, *v);
                    s
                })
            })
            .collect();
    }
    let rules: Vec<Value> = all
        .iter()
        .map(|s| {
            let state = json!({"id": block, "properties": s});
            json!({
                "input_predicate": {"predicate_type": "minecraft:blockstate_match", "block_state": state},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": state,
                "block_entity_modifier": {"type": "horizon:edit", "remove": remove, "data": nbt(s)},
            })
        })
        .collect();
    json!({"processor_type": "minecraft:rule", "rules": rules})
}

/// An entity's NBT in 26.3's shape, in place: 1.21.1's `ArmorItems` (feet
/// to head), `HandItems` (main hand, off hand) and their drop chances become
/// `equipment` and `drop_chances` by slot; empty items and the default drop
/// chance (0.085) are left out, as 26.3 saves them. Whether it changed.
pub fn upgrade_entity(entity: &mut Value) -> bool {
    let Some(o) = entity.as_object_mut() else {
        return false;
    };
    const ARMOR: [&str; 4] = ["feet", "legs", "chest", "head"];
    const HANDS: [&str; 2] = ["mainhand", "offhand"];
    let mut changed = false;
    for (items, chances, slots) in [
        ("ArmorItems", "ArmorDropChances", &ARMOR[..]),
        ("HandItems", "HandDropChances", &HANDS[..]),
    ] {
        if let Some(Value::Array(list)) = o.remove(items) {
            changed = true;
            for (slot, item) in slots.iter().zip(list) {
                if item.get("id").is_some() {
                    let equipment = o.entry("equipment").or_insert_with(|| json!({}));
                    equipment[*slot] = item;
                }
            }
        }
        if let Some(Value::Array(list)) = o.remove(chances) {
            changed = true;
            for (slot, chance) in slots.iter().zip(list) {
                if chance.as_f64().is_some_and(|c| (c - 0.085).abs() > 1e-6) {
                    let drop = o.entry("drop_chances").or_insert_with(|| json!({}));
                    drop[*slot] = chance;
                }
            }
        }
    }
    changed
}

/// A 1.21.1 trial spawner config (`trial_spawner/`) in 26.3's shape, in
/// place: its mobs' NBT ([`upgrade_entity`]). Whether it changed.
pub fn upgrade_trial_spawner(config: &mut Value) -> bool {
    let mut changed = false;
    for p in config["spawn_potentials"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        changed |= upgrade_entity(&mut p["data"]["entity"]);
    }
    changed
}

/// A block state as 26.3 writes it.
fn block(id: &str) -> Value {
    json!({"id": id})
}

/// Better Desert Temples' placeholder blocks (glass, concrete and wool in
/// the templates) as the mod's processors turn them: the fixed ones as
/// vanilla's `rule`, the random ones as the game's `horizon:pick`, with the
/// mod's bounds on one `nextFloat` (YUNG's BlockStateRandomizer adds its
/// chances up in floats). The ones that read or change the world around
/// (pillars, Diorite's waterlogging) stay dropped.
fn desert_temple(kind: &str) -> Option<Value> {
    let swap = |from: &str, to: &str| {
        json!({"processor_type": "minecraft:rule", "rules": [{
            "input_predicate": {"predicate_type": "minecraft:block_match", "block": from},
            "location_predicate": {"predicate_type": "minecraft:always_true"},
            "output_state": block(to)}]})
    };
    let pick = |from: &str, picks: Vec<Value>, default: Value| json!({"processor_type": "horizon:pick", "block": from, "picks": picks, "default": default});
    // YUNG's BlockStateRandomizer: an entry wins below the sum of the
    // chances so far, summed as floats.
    let randomizer = |entries: &[(f32, &str)]| {
        let mut bottom = 0f32;
        entries
            .iter()
            .map(|(chance, id)| {
                bottom += chance;
                json!({"below": bottom as f64, "state": block(id)})
            })
            .collect::<Vec<_>>()
    };
    let at = |below: f32, id: &str| json!({"below": below as f64, "state": block(id)});
    // Sandstone stairs, upside down, facing Plane.HORIZONTAL.getRandomDirection.
    let stairs = || {
        let states: Vec<Value> = ["north", "east", "south", "west"]
            .iter()
            .map(|facing| {
                json!({"id": "minecraft:sandstone_stairs", "properties": {
                    "facing": facing, "half": "top", "shape": "straight", "waterlogged": "false"}})
            })
            .collect();
        json!({"states": states})
    };
    Some(match kind.strip_prefix("betterdeserttemples:")? {
        "blue_concrete_processor" => swap("minecraft:blue_concrete", "minecraft:blue_terracotta"),
        "yellow_concrete_processor" => {
            swap("minecraft:yellow_concrete", "minecraft:yellow_terracotta")
        }
        "red_wool_processor" => swap("minecraft:red_wool", "minecraft:tnt"),
        "white_stained_glass_processor" => pick(
            "minecraft:white_stained_glass",
            randomizer(&[(0.4, "minecraft:cobweb")]),
            json!({"state": block("minecraft:air")}),
        ),
        "polished_diorite_processor" => pick(
            "minecraft:polished_diorite",
            randomizer(&[(0.6, "minecraft:cut_sandstone")]),
            json!({"state": block("minecraft:sandstone")}),
        ),
        "orange_stained_glass_processor" => {
            let mut sus = at(0.01, "minecraft:suspicious_sand");
            sus["nbt"] = json!({"LootTable": "minecraft:archaeology/desert_pyramid"});
            let mut stairs = stairs();
            stairs["below"] = (0.45f32 as f64).into();
            pick(
                "minecraft:orange_stained_glass",
                vec![
                    sus,
                    at(0.1, "minecraft:sand"),
                    at(0.2, "minecraft:sandstone"),
                    stairs,
                ],
                json!({"state": block("minecraft:cut_sandstone")}),
            )
        }
        "red_stained_glass_processor" => pick(
            "minecraft:red_stained_glass",
            vec![at(0.3, "minecraft:sand"), at(0.75, "minecraft:sandstone")],
            stairs(),
        ),
        "pot_processor" => {
            // Four sherds, one float each; the pot back to its default state.
            let values: Vec<Value> = [
                (0.05f32, "archer"),
                (0.1, "miner"),
                (0.15, "prize"),
                (0.2, "skull"),
            ]
            .iter()
            .map(|(below, sherd)| {
                json!({"below": *below as f64, "value": format!("minecraft:{sherd}_pottery_sherd")})
            })
            .collect();
            pick(
                "minecraft:decorated_pot",
                Vec::new(),
                json!({"state": block("minecraft:decorated_pot"), "lists": [{
                    "key": "sherds", "count": 4, "values": values, "default": "minecraft:brick"}]}),
            )
        }
        _ => return None,
    })
}

/// [`as_data`]'s `horizon:spawner_mobs` for the mods' spawner randomizing
/// processors, whose fields have the same names and defaults.
fn spawner_mobs(p: &Value, mobs: Vec<Value>, rules: Option<Value>) -> Value {
    let mut out = json!({"processor_type": "horizon:spawner_mobs", "mobs": mobs});
    for key in [
        "delay",
        "min_spawn_delay",
        "max_spawn_delay",
        "spawn_count",
        "max_nearby_entities",
        "required_player_range",
        "spawn_range",
    ] {
        if let Some(n) = p.get(key).filter(|n| n.is_i64()) {
            out[key] = n.clone();
        }
    }
    if let Some(r) = rules {
        out["custom_spawn_rules"] = r;
    }
    if let Some(b) = p.get("spawner_replacement_block") {
        out["replacement"] = b.clone();
    }
    out
}

/// Repurposed Structures' `custom_spawn_rules` from its light level ranges:
/// `Some(None)` without either, `None` when one does not read. The sky
/// light's upper bound is written as `max_exclusive`, as the mod does.
fn repurposed_light(p: &Value) -> Option<Option<Value>> {
    let range = |key: &str| -> Option<Option<(i64, i64)>> {
        match p.get(key) {
            None => Some(None),
            Some(Value::Number(n)) => n.as_i64().map(|n| Some((n, n))),
            Some(v) => Some(Some((
                v["min_inclusive"].as_i64()?,
                v["max_inclusive"].as_i64()?,
            ))),
        }
    };
    let (block, sky) = (
        range("valid_block_light_level")?,
        range("valid_sky_light_level")?,
    );
    if block.is_none() && sky.is_none() {
        return Some(None);
    }
    let mut rules = json!({});
    if let Some((min, max)) = block {
        rules["block_light_limit"] = json!({"min_inclusive": min, "max_inclusive": max});
    }
    if let Some((min, max)) = sky {
        rules["sky_light_limit"] = json!({"min_inclusive": min, "max_exclusive": max});
    }
    Some(Some(rules))
}

/// An item as 26.3 saves it.
fn item(id: &str) -> Value {
    json!({"id": format!("minecraft:{id}"), "count": 1})
}

/// YUNG's processors that make a spawner of a fixed mob (YUNG's API's
/// `MobSpawnerData`), as vanilla's `rule`: every `input` block becomes a
/// spawner with that NBT.
fn yung_spawner(kind: &str, p: &Value) -> Option<Value> {
    const SPAWNER: &str = "minecraft:spawner";
    // (input block, mob, fields other than the defaults, whether the mob is
    // also the only potential, its equipment)
    let (input, mob, fields, potential, equipment): (_, _, &[(&str, i64)], _, _) = match kind {
        "betterdungeons:mob_spawner_processor" => {
            (SPAWNER, p["spawner_mob"].as_str()?, &[], true, None)
        }
        "betterdungeons:zombie_mob_spawner_processor" => (
            SPAWNER,
            "minecraft:zombie",
            &[("MaxNearbyEntities", 8)],
            true,
            None,
        ),
        "betterdungeons:skeleton_mob_spawner_processor" => (
            SPAWNER,
            "minecraft:skeleton",
            &[
                ("RequiredPlayerRange", 18),
                ("MaxNearbyEntities", 8),
                ("MaxSpawnDelay", 650),
            ],
            true,
            None,
        ),
        "betterdungeons:small_nether_dungeon_mob_spawner_processor" => {
            let mob = p["spawner_mob"].as_str()?;
            // Wither skeletons get netherite armor that never drops and a
            // stone sword (with the mod's default config, they and blazes
            // keep their loot).
            let armor = (mob == "minecraft:wither_skeleton").then(|| {
                json!({
                    "equipment": {
                        "feet": item("netherite_boots"),
                        "legs": item("netherite_leggings"),
                        "chest": item("netherite_chestplate"),
                        "head": item("netherite_helmet"),
                        "mainhand": item("stone_sword"),
                    },
                    "drop_chances": {"feet": 0.0, "legs": 0.0, "chest": 0.0, "head": 0.0},
                })
            });
            (SPAWNER, mob, &[], true, armor)
        }
        "betterdungeons:zombie_tombstone_spawner_processor" => (
            "minecraft:black_stained_glass",
            "minecraft:skeleton",
            &[],
            false,
            Some(json!({"equipment": {"mainhand": item("iron_sword")}})),
        ),
        "betterdeserttemples:gravel_processor" => (
            "minecraft:gravel",
            "minecraft:husk",
            &[("MaxNearbyEntities", 8), ("RequiredPlayerRange", 24)],
            false,
            None,
        ),
        // An empty hand list in 1.21.1: the skeletons spawn without a bow,
        // since a spawn entry with more than its id is not finalized.
        "betterdeserttemples:bone_block_processor" => (
            "minecraft:bone_block",
            "minecraft:skeleton",
            &[("RequiredPlayerRange", 32)],
            false,
            Some(json!({"equipment": {}})),
        ),
        "betterdeserttemples:infested_cracked_stone_bricks_processor" => (
            "minecraft:infested_cracked_stone_bricks",
            "minecraft:silverfish",
            &[("RequiredPlayerRange", 24)],
            false,
            None,
        ),
        // 1.21.1 ignores the items' old `Count` and `tag` (the shield's
        // pattern, the boots' dye): plain items.
        "betterdeserttemples:yellow_wool_processor" => (
            "minecraft:yellow_wool",
            "minecraft:husk",
            &[("MaxNearbyEntities", 8), ("RequiredPlayerRange", 24)],
            false,
            Some(json!({"equipment": {
                "mainhand": item("golden_sword"),
                "offhand": item("shield"),
                "feet": item("leather_boots"),
                "legs": item("leather_leggings"),
                "chest": item("golden_chestplate"),
                "head": item("chainmail_helmet"),
            }})),
        ),
        _ => return None,
    };
    let mut entity = json!({"id": mob});
    if let Some(Value::Object(extra)) = equipment {
        entity.as_object_mut()?.extend(extra);
    }
    let potentials = match potential {
        true => json!([{"data": {"entity": entity.clone()}, "weight": 1}]),
        false => json!([]),
    };
    let mut nbt = json!({
        "Delay": 20, "MinSpawnDelay": 200, "MaxSpawnDelay": 800, "SpawnCount": 4,
        "MaxNearbyEntities": 6, "RequiredPlayerRange": 16, "SpawnRange": 4,
        "SpawnData": {"entity": entity},
        "SpawnPotentials": potentials,
    });
    for (key, value) in fields {
        nbt[*key] = (*value).into();
    }
    Some(json!({
        "processor_type": "minecraft:rule",
        "rules": [{
            "input_predicate": {"predicate_type": "minecraft:block_match", "block": input},
            "location_predicate": {"predicate_type": "minecraft:always_true"},
            "output_state": {"Name": SPAWNER},
            "block_entity_modifier": {"type": "minecraft:append_static", "data": nbt},
        }],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> ProcessorData {
        let mut d = ProcessorData::default();
        d.spawners.insert(
            "repurposed_structures:dungeons/badlands".into(),
            vec![
                ("minecraft:husk".into(), 10.),
                ("minecraft:cave_spider".into(), 3.),
            ],
        );
        d.replace_vanilla.insert(("mvs".into(), "village".into()));
        d
    }

    #[test]
    fn spawner_processors_become_data() {
        let mut list = json!({"processors": [
            {"processor_type": "moogs_structures:spawner_randomizing_processor",
             "weighted_entities": [{"entity": "minecraft:blaze", "weight": 2, "nbt": {"Health": 5}}],
             "spawn_count": 2, "spawner_replacement_block": {"Name": "minecraft:stone"}},
            {"processor_type": "repurposed_structures:spawner_randomizing_processor",
             "rs_spawner_resourcelocation": "repurposed_structures:dungeons/badlands",
             "valid_block_light_level": {"min_inclusive": 0, "max_inclusive": 7}},
            {"processor_type": "repurposed_structures:spawner_randomizing_processor",
             "rs_spawner_resourcelocation": "repurposed_structures:missing"},
            {"processor_type": "minecraft:gravity", "heightmap": "WORLD_SURFACE_WG", "offset": 0},
            {"processor_type": "moogs_structures:vanilla_loot_swap_processor",
             "modid": "mns", "vanilla_key": "fortress", "loot_table_mapping": {}},
            {"processor_type": "moogs_structures:vanilla_loot_swap_processor",
             "modid": "mvs", "vanilla_key": "village", "loot_table_mapping": {}}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        assert_eq!(p.len(), 5, "{p:#?}");
        assert_eq!(
            p[0],
            json!({"processor_type": "horizon:spawner_mobs",
                   "mobs": [{"entity": {"Health": 5, "id": "minecraft:blaze"}, "weight": 2.0}],
                   "spawn_count": 2, "replacement": {"Name": "minecraft:stone"}})
        );
        assert_eq!(
            p[1],
            json!({"processor_type": "horizon:spawner_mobs",
                   "mobs": [{"entity": {"id": "minecraft:husk"}, "weight": 10.0},
                            {"entity": {"id": "minecraft:cave_spider"}, "weight": 3.0}],
                   "custom_spawn_rules": {"block_light_limit": {"min_inclusive": 0, "max_inclusive": 7}}})
        );
        // Unknown list, vanilla processor, and a swap whose preset is on: kept
        // (the Java ones are dropped later); the swap that is off is gone.
        assert_eq!(
            p[2]["rs_spawner_resourcelocation"],
            "repurposed_structures:missing"
        );
        assert_eq!(p[3]["processor_type"], "minecraft:gravity");
        assert_eq!(p[4]["modid"], "mvs");
        assert_eq!(
            carried,
            BTreeMap::from([
                ("moogs_structures:spawner_randomizing_processor".into(), 1),
                ("moogs_structures:vanilla_loot_swap_processor".into(), 1),
                (
                    "repurposed_structures:spawner_randomizing_processor".into(),
                    1
                ),
            ])
        );
    }

    #[test]
    fn yungs_spawners_become_rules() {
        let mut pool = json!({"elements": [{"element": {"processors": {"processors": [
            {"processor_type": "betterdungeons:skeleton_mob_spawner_processor"},
            {"processor_type": "betterdeserttemples:bone_block_processor"},
            {"processor_type": "betterdungeons:small_nether_dungeon_mob_spawner_processor",
             "spawner_mob": "minecraft:wither_skeleton"},
            {"processor_type": "betterdungeons:candle_processor"}]}}}]});
        let mut carried = BTreeMap::new();
        assert!(carry("template_pool", &mut pool, &data(), &mut carried));
        let p = &pool["elements"][0]["element"]["processors"]["processors"];
        let rule = |i: usize| &p[i]["rules"][0];
        assert_eq!(rule(0)["input_predicate"]["block"], "minecraft:spawner");
        assert_eq!(
            rule(0)["output_state"],
            json!({"Name": "minecraft:spawner"})
        );
        let nbt = &rule(0)["block_entity_modifier"]["data"];
        assert_eq!(
            (
                nbt["RequiredPlayerRange"].clone(),
                nbt["MaxNearbyEntities"].clone()
            ),
            (json!(18), json!(8))
        );
        assert_eq!(nbt["MaxSpawnDelay"], 650);
        assert_eq!(
            nbt["SpawnPotentials"],
            json!([{"data": {"entity": {"id": "minecraft:skeleton"}}, "weight": 1}])
        );
        // Bone blocks become bowless skeleton spawners with no potentials.
        let bone = &rule(1)["block_entity_modifier"]["data"];
        assert_eq!(rule(1)["input_predicate"]["block"], "minecraft:bone_block");
        assert_eq!(
            bone["SpawnData"],
            json!({"entity": {"id": "minecraft:skeleton", "equipment": {}}})
        );
        assert_eq!(bone["SpawnPotentials"], json!([]));
        let wither = &rule(2)["block_entity_modifier"]["data"]["SpawnData"]["entity"];
        assert_eq!(
            wither["equipment"]["head"]["id"],
            "minecraft:netherite_helmet"
        );
        assert_eq!(wither["drop_chances"]["feet"], 0.0);
        assert_eq!(p[3]["processor_type"], "betterdungeons:candle_processor");
        assert_eq!(carried.values().sum::<usize>(), 3);
    }

    #[test]
    fn trial_spawners_and_vaults_drop_their_saved_state() {
        let mut list = json!({"processors": [
            {"processor_type": "moogs_structures:trial_spawner_randomizing_processor",
             "normal_config": "mns:arena/normal", "ominous_config": "mns:arena/ominous"},
            {"processor_type": "moogs_structures:vault_randomizing_processor",
             "loot_table": "mns:vaults/main", "ominous_loot_table": "mns:vaults/ominous"}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        let rules = |i: usize| p[i]["rules"].as_array().unwrap().clone();
        // Every state of the block: 6 x 2 trial spawners, 4 x 4 x 2 vaults.
        assert_eq!((rules(0).len(), rules(1).len()), (12, 32));
        let spawner = &rules(0)[0];
        assert_eq!(
            spawner["input_predicate"]["block_state"],
            spawner["output_state"]
        );
        assert_eq!(
            spawner["block_entity_modifier"],
            json!({"type": "horizon:edit",
                   "remove": ["server_data", "shared_data", "spawn_data",
                              "cooldown_end_at_tick", "next_mob_spawns_at"],
                   "data": {"normal_config": "mns:arena/normal",
                            "ominous_config": "mns:arena/ominous"}})
        );
        for rule in rules(1) {
            let ominous = rule["output_state"]["properties"]["ominous"] == "true";
            let (table, key) = match ominous {
                true => ("mns:vaults/ominous", "minecraft:ominous_trial_key"),
                false => ("mns:vaults/main", "minecraft:trial_key"),
            };
            assert_eq!(
                rule["block_entity_modifier"],
                json!({"type": "horizon:edit", "remove": ["server_data", "shared_data"],
                       "data": {"config": {"loot_table": table,
                                           "key_item": {"id": key, "count": 1}}}})
            );
        }
        assert_eq!(carried.len(), 2);
    }

    #[test]
    fn trial_spawner_mobs_take_26_3s_equipment() {
        let mut config = json!({"spawn_potentials": [{"weight": 1, "data": {"entity": {
            "id": "minecraft:skeleton",
            "ArmorItems": [{}, {}, {}, {"id": "minecraft:iron_helmet", "count": 1}],
            "ArmorDropChances": [0.085, 0.085, 0.085, 0.0],
            "HandItems": [{"id": "minecraft:bow", "count": 1}, {}],
            "HandDropChances": [0.085, 0.085]}}}]});
        assert!(upgrade_trial_spawner(&mut config));
        assert_eq!(
            config["spawn_potentials"][0]["data"]["entity"],
            json!({"id": "minecraft:skeleton",
                   "equipment": {"head": {"id": "minecraft:iron_helmet", "count": 1},
                                 "mainhand": {"id": "minecraft:bow", "count": 1}},
                   "drop_chances": {"head": 0.0}})
        );
        assert!(!upgrade_trial_spawner(&mut config));
    }

    #[test]
    fn desert_temple_placeholders_become_rules_and_picks() {
        let mut list = json!({"processors": [
            {"processor_type": "betterdeserttemples:red_wool_processor"},
            {"processor_type": "betterdeserttemples:white_stained_glass_processor"},
            {"processor_type": "betterdeserttemples:orange_stained_glass_processor"},
            {"processor_type": "betterdeserttemples:pot_processor"},
            {"processor_type": "betterdeserttemples:quartz_pillar_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        assert_eq!(
            p[0]["rules"][0]["output_state"],
            json!({"id": "minecraft:tnt"})
        );
        assert_eq!(p[1]["processor_type"], "horizon:pick");
        assert_eq!(p[1]["picks"][0]["below"], 0.4f32 as f64);
        assert_eq!(p[1]["default"]["state"]["id"], "minecraft:air");
        let glass = &p[2]["picks"];
        assert_eq!(glass[0]["state"]["id"], "minecraft:suspicious_sand");
        assert_eq!(
            glass[0]["nbt"]["LootTable"],
            "minecraft:archaeology/desert_pyramid"
        );
        assert_eq!(glass[3]["states"].as_array().unwrap().len(), 4);
        assert_eq!(glass[3]["below"], 0.45f32 as f64);
        assert_eq!(p[3]["default"]["lists"][0]["count"], 4);
        // The pillar reads the world below: still Java, still dropped later.
        assert_eq!(
            p[4]["processor_type"],
            "betterdeserttemples:quartz_pillar_processor"
        );
        assert_eq!(carried.values().sum::<usize>(), 4);
    }

    #[test]
    fn repurposed_waterlogging_follows_the_world() {
        let mut list = json!({"processors": [{"processor_type":
            "repurposed_structures:waterlogging_when_replacing_water_processor"},
            {"processor_type": "repurposed_structures:mineshaft_sky_view_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry(
            "processor_list",
            &mut list,
            &ProcessorData::default(),
            &mut carried
        ));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "horizon:waterlog_in_world_water"},
                {"processor_type": "horizon:mineshaft_sky_view"}])
        );
    }

    #[test]
    fn small_dungeon_chests_are_capped() {
        let mut list = json!({"processors": [
            {"processor_type": "betterdungeons:small_dungeon_chest_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let cap = &list["processors"][0];
        assert_eq!(cap["processor_type"], "horizon:count_cap");
        assert_eq!(
            (cap["min"].as_u64(), cap["max"].as_u64()),
            (Some(1), Some(2))
        );
        assert_eq!(cap["replacement"], json!({"id": "minecraft:cave_air"}));
    }

    #[test]
    fn lithostitched_block_swaps_become_the_games() {
        let mut list = json!({"processors": [
            {"processor_type": "lithostitched:block_swap",
             "blocks": {"minecraft:cobblestone": "minecraft:mossy_cobblestone"}},
            {"processor_type": "lithostitched:block_swap", "blocks": []}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        assert_eq!(
            p[0],
            json!({"processor_type": "horizon:block_swap",
                   "blocks": {"minecraft:cobblestone": "minecraft:mossy_cobblestone"}})
        );
        // A map that is not one stays Java (dropped later).
        assert_eq!(p[1]["processor_type"], "lithostitched:block_swap");
    }

    #[test]
    fn modular_compat_runs_its_list_only_with_its_mod() {
        let mut data = ProcessorData::default();
        let swap = json!({"processor_type": "minecraft:rule", "rules": []});
        data.lists
            .insert("v:house_ns".into(), json!({"processors": [swap.clone()]}));
        let compat = |m: &str| json!({"processor_type": "ctov:modular_compat", "modid": m, "reference": "v:house_ns"});
        let gravity = json!({"processor_type": "minecraft:gravity"});
        data.lists.insert(
            "v:house".into(),
            json!({"processors": [compat("ns"), compat("absent"), gravity.clone()]}),
        );
        data.mark_loaded(&|m| m == "ns");
        assert_eq!(data.compat_loaded, BTreeSet::from(["ns".to_owned()]));
        let mut v = json!({"processors": [compat("ns"), compat("absent"), gravity.clone()]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut v, &data, &mut carried));
        assert_eq!(v, json!({"processors": [swap, gravity]}));
        assert_eq!(carried["ctov:modular_compat"], 2);
    }

    #[test]
    fn lithostitched_references_are_written_in() {
        let mut data = data();
        let gravity = json!({"processor_type": "minecraft:gravity", "heightmap": "WORLD_SURFACE_WG", "offset": 0});
        data.lists
            .insert("v:road".into(), json!({"processors": [gravity.clone()]}));
        data.lists.insert(
            "v:house".into(),
            json!({"processors": [
                {"processor_type": "lithostitched:reference", "processor_lists": "v:road"},
                {"processor_type": "lithostitched:block_swap", "blocks": {"minecraft:dirt": "minecraft:grass_block"}}]}),
        );
        let mut list = json!({"processors": [
            {"processor_type": "lithostitched:reference", "processor_lists": ["v:house", "v:road"]},
            {"processor_type": "lithostitched:reference", "processor_lists": "#v:roads"},
            {"processor_type": "lithostitched:reference", "processor_lists": ["v:nowhere"]}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data, &mut carried));
        let p = list["processors"].as_array().unwrap();
        // House (its own reference to road first, then its swap), then road;
        // the swap inside is carried too.
        assert_eq!(p[0], gravity);
        assert_eq!(p[1]["processor_type"], "horizon:block_swap");
        assert_eq!(p[2], gravity);
        // A tag or a list the mod lacks stays, to be dropped.
        assert_eq!(p[3]["processor_lists"], "#v:roads");
        assert_eq!(p[4]["processor_lists"], json!(["v:nowhere"]));
        assert_eq!(carried["lithostitched:reference"], 2);
    }

    #[test]
    fn lithostitched_random_and_condition_become_the_games() {
        let mut data = data();
        let swap = json!({"processor_type": "lithostitched:block_swap",
            "blocks": {"minecraft:dirt": "minecraft:grass_block"}});
        data.lists
            .insert("v:green".into(), json!({"processors": [swap.clone()]}));
        let mut list = json!({"processors": [
            {"processor_type": "lithostitched:apply_random", "mode": "per_piece", "processor_lists": [
                {"weight": 3, "data": "minecraft:empty"},
                {"weight": 1, "data": {"processors": [
                    {"processor_type": "lithostitched:reference", "processor_lists": "v:green"},
                    {"processor_type": "v:java"}]}}]},
            {"processor_type": "lithostitched:apply_random", "mode": "per_block",
             "processor_lists": ["v:green", "v:red"]},
            {"processor_type": "lithostitched:apply_random", "mode": "per_block",
             "processor_lists": "#v:colours"},
            {"processor_type": "lithostitched:condition",
             "if_true": [{"type": "lithostitched:matching_blocks", "blocks": "minecraft:cobblestone"}],
             "then": swap, "else": {"processor_type": "lithostitched:set_block", "state_provider":
                {"type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:stone"}}}},
            {"processor_type": "lithostitched:condition",
             "if_true": {"type": "lithostitched:matching_biomes", "biomes": "#c:is_cold"},
             "then": []}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data, &mut carried));
        let p = list["processors"].as_array().unwrap();
        let green = json!({"processor_type": "horizon:block_swap",
            "blocks": {"minecraft:dirt": "minecraft:grass_block"}});
        // Inline lists are carried; Java left in them is dropped later.
        assert_eq!(
            p[0],
            json!({"processor_type": "horizon:apply_random", "mode": "per_piece", "processor_lists": [
                {"weight": 3, "data": "minecraft:empty"},
                {"weight": 1, "data": {"processors": [green.clone(), {"processor_type": "v:java"}]}}]})
        );
        assert_eq!(
            p[1]["processor_lists"],
            json!([{"weight": 1, "data": "v:green"}, {"weight": 1, "data": "v:red"}])
        );
        assert_eq!(p[2]["processor_type"], "lithostitched:apply_random");
        assert_eq!(
            p[3],
            json!({"processor_type": "horizon:condition",
                "if_true": [{"type": "lithostitched:matching_blocks", "blocks": "minecraft:cobblestone"}],
                "then": [green], "else": [{"processor_type": "horizon:set_block", "state_provider":
                    {"type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:stone"}}}]})
        );
        assert_eq!(p[4]["processor_type"], "lithostitched:condition");
        assert_eq!(carried["lithostitched:apply_random"], 2);
        assert_eq!(carried["lithostitched:condition"], 1);
    }

    #[test]
    fn repurposed_rule_tests_become_the_games() {
        let rule = |input: Value, position: Value| {
            json!({"input_predicate": input, "location_predicate": {"predicate_type":
                "repurposed_structures:matter_phase_rule_test", "phase_to_test_for": "AIR"},
                "position_predicate": position, "output_state": {"id": "minecraft:lava"}})
        };
        let mut pool = json!({"elements": [{"weight": 1, "element": {
            "element_type": "minecraft:single_pool_element", "processors": {"processors": [
                {"processor_type": "minecraft:rule", "rules": [rule(
                    json!({"predicate_type": "minecraft:always_true"}),
                    json!({"predicate_type": "repurposed_structures:y_value_pos_rule_test",
                        "min_y_value": 0, "max_y_value": 31}))]},
                {"processor_type": "minecraft:capped", "limit": 2, "delegate": {
                    "processor_type": "minecraft:rule", "rules": [rule(
                        json!({"predicate_type": "minecraft:block_match", "block": "minecraft:rail"}),
                        json!({"predicate_type":
                            "repurposed_structures:piece_origin_axis_aligned_linear_pos_rule_test",
                            "axis": "y", "min_dist": 0, "max_dist": 1}))]}}]}}}]});
        let mut carried = BTreeMap::new();
        assert!(rule_tests("template_pool", &mut pool, &mut carried));
        let list = &pool["elements"][0]["element"]["processors"]["processors"];
        let first = &list[0]["rules"][0];
        assert_eq!(
            first["location_predicate"]["predicate_type"],
            "horizon:matter_phase"
        );
        assert_eq!(first["location_predicate"]["phase_to_test_for"], "AIR");
        assert_eq!(
            first["position_predicate"]["predicate_type"],
            "horizon:y_value"
        );
        assert_eq!(first["position_predicate"]["max_y_value"], 31);
        let delegated = &list[1]["delegate"]["rules"][0];
        assert_eq!(
            delegated["position_predicate"]["predicate_type"],
            "horizon:piece_origin_axis_aligned_linear_pos"
        );
        assert_eq!(
            delegated["input_predicate"]["predicate_type"],
            "minecraft:block_match"
        );
        assert_eq!(carried.values().sum::<usize>(), 4);
        assert_eq!(carried["repurposed_structures:matter_phase_rule_test"], 2);
        // A file holding none is left as it is.
        assert!(!rule_tests("template_pool", &mut pool, &mut carried));
        assert!(!rule_tests(
            "structure",
            &mut json!({"predicate_type":
            "repurposed_structures:y_value_pos_rule_test"}),
            &mut carried
        ));
    }

    #[test]
    fn ocean_monuments_become_the_games() {
        let names = [
            "random_prismarine_slab_decoration_processor",
            "random_dark_prismarine_slab_decoration_processor",
            "sand_gravel_processor",
            "random_oxidization_processor",
            "structure_void_processor",
            "air_processor",
            "waterlog_processor",
            "seagrass_processor",
            "random_sponge_processor",
            "leg_processor",
        ];
        let processors: Vec<Value> = names
            .iter()
            .map(|n| json!({"processor_type": format!("betteroceanmonuments:{n}")}))
            .chain([json!({"processor_type": "betterwitchhuts:fence_leg_processor"})])
            .collect();
        let mut list = json!({"processors": processors});
        let mut carried = BTreeMap::new();
        assert!(carry(
            "processor_list",
            &mut list,
            &ProcessorData::default(),
            &mut carried
        ));
        let p = list["processors"].as_array().unwrap();
        let types: Vec<&str> = p
            .iter()
            .map(|p| p["processor_type"].as_str().unwrap())
            .collect();
        let mut want = vec!["horizon:pick", "horizon:pick", "minecraft:rule"];
        want.extend(["horizon:pick"; 4]);
        want.extend([
            "horizon:keep_world_block",
            "horizon:flood_air",
            "horizon:waterlog",
            "minecraft:rule",
            "horizon:pick",
            "horizon:yung_pillar",
            "horizon:yung_pillar",
        ]);
        assert_eq!(types, want);
        assert_eq!(carried.values().sum::<usize>(), 11);
        // Copper stairs keep their properties, each chance on a new float.
        let stairs = &p[5];
        assert_eq!(stairs["block"], "minecraft:oxidized_cut_copper_stairs");
        assert_eq!(
            stairs["picks"][1]["state"]["id"],
            "minecraft:weathered_cut_copper_stairs"
        );
        assert_eq!(
            (stairs["redraw"].clone(), stairs["keep_properties"].clone()),
            (json!(true), json!(true))
        );
        assert_eq!(p[0]["picks"][0]["state"]["properties"]["type"], "top");
        assert_eq!(p[1]["default"]["state"]["id"], "minecraft:water");
        assert_eq!(
            p[12]["target_block_output"]["defaultBlockState"]["id"],
            "minecraft:prismarine_bricks"
        );
        assert_eq!(p[13]["keep_properties"], true);
    }

    #[test]
    fn repurposed_structure_voids_keep_the_world() {
        let mut list = json!({"processors": [
            {"processor_type": "repurposed_structures:structure_void_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry(
            "processor_list",
            &mut list,
            &ProcessorData::default(),
            &mut carried
        ));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "minecraft:block_ignore",
                "blocks": ["minecraft:structure_void"]}])
        );
        assert_eq!(carried["repurposed_structures:structure_void_processor"], 1);
    }

    #[test]
    fn pillars_become_the_games() {
        let mut d = ProcessorData::default();
        // Repurposed's city list names itself as its pillar's list.
        let city = json!({"processors": [
            {"processor_type": "repurposed_structures:pillar_processor", "direction": "down",
             "pillar_length": 15, "pillar_processor_list": "repurposed_structures:cities/x",
             "pillar_trigger_and_replacements": [{"trigger": {"Name": "minecraft:red_stained_glass"},
                 "replacement": {"Name": "minecraft:cobblestone"}}]},
            {"processor_type": "minecraft:block_rot", "integrity": 0.5}]});
        d.lists
            .insert("repurposed_structures:cities/x".into(), city.clone());
        let mut list = city;
        list["processors"].as_array_mut().unwrap().extend([
            json!({"processor_type": "repurposed_structures:bottom_pillar_processor",
                   "pillar_processor_list": "minecraft:empty"}),
            json!({"processor_type": "moogs_structures:close_off_fluid_sources_processor",
                   "weighted_list_of_replacement_blocks": [{"block": "minecraft:stone", "weight": 1}]}),
            json!({"processor_type": "repurposed_structures:close_off_air_sources_processor",
                   "weighted_list_of_replacement_blocks": [{"block": "minecraft:prismarine", "weight": 1}]}),
            json!({"processor_type": "betterfortresses:pillar_processor",
                   "target_block": {"Name": "minecraft:orange_terracotta"},
                   "target_block_output": {"defaultBlockState": {"Name": "minecraft:red_nether_bricks"}, "entries": []},
                   "pillar_states": {"defaultBlockState": {"Name": "minecraft:nether_bricks"}, "entries": []}}),
            json!({"processor_type": "betterfortresses:stair_pillar_processor"}),
        ]);
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &d, &mut carried));
        let p = list["processors"].as_array().unwrap();
        let types: Vec<&str> = p
            .iter()
            .map(|p| p["processor_type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            [
                "horizon:pillar",
                "minecraft:block_rot",
                "horizon:bottom_pillar",
                "horizon:close_off_fluid_sources",
                "horizon:close_off_air_sources",
                "horizon:yung_pillar",
                "horizon:stair_pillar"
            ]
        );
        // The named list is written in once; its own pillar loses its list.
        let inner = &p[0]["pillar_processor_list"]["processors"];
        assert_eq!(inner[0]["processor_type"], "horizon:pillar");
        assert!(inner[0].get("pillar_processor_list").is_none());
        assert_eq!(
            inner[0]["pillar_trigger_and_replacements"],
            p[0]["pillar_trigger_and_replacements"]
        );
        assert_eq!(inner[1]["processor_type"], "minecraft:block_rot");
        assert!(p[2].get("pillar_processor_list").is_none());
        assert_eq!(
            p[4]["weighted_list_of_replacement_blocks"],
            json!([{"block": "minecraft:prismarine", "weight": 1}])
        );
        assert_eq!(p[6]["pillar"], "minecraft:red_nether_bricks");
        assert_eq!(carried["repurposed_structures:pillar_processor"], 2);
        assert_eq!(carried["betterfortresses:stair_pillar_processor"], 1);
    }
}
