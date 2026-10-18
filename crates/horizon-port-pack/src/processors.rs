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
}

impl ProcessorData {
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
            if spawners.is_none() && rest != "moogs_structures/replace_vanilla" {
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
        let mut changed = false;
        let mut out = Vec::with_capacity(list.len());
        for p in list.drain(..) {
            let kind = p["processor_type"].as_str().unwrap_or_default().to_owned();
            match as_data(&kind, &p, data) {
                Some(replacement) => {
                    *carried.entry(kind).or_default() += 1;
                    out.extend(replacement);
                    changed = true;
                }
                None => out.push(p),
            }
        }
        *list = out;
        changed
    })
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

/// The data form of a Java processor: the processors to put in its place
/// (none for a no-op), or `None` when it has none.
fn as_data(kind: &str, p: &Value, data: &ProcessorData) -> Option<Vec<Value>> {
    let (ns, _) = kind.split_once(':')?;
    Some(match kind {
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
        _ if ns == "betterdungeons" || ns == "betterdeserttemples" => vec![yung_spawner(kind, p)?],
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
}
