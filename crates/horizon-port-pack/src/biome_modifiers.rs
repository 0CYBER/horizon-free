//! Biome modifiers of mods' own types whose fields say everything the game
//! needs, written as NeoForge's (`add_features`, `remove_features`,
//! `add_spawns`). The game reads only NeoForge's and skips the others, so the
//! ores, plants, nests and fish they add never appeared. Worldgen is saved
//! into chunks, so a type is only written when NeoForge's selectors (and
//! Horizon's `horizon:climate`) say exactly which biomes the mod's code picks;
//! one that can't be said exactly stays outside the carry ([`inexact`]). A
//! type whose biomes or features are in the mod's Java alone
//! (`cataclysm:cataclysm_mob_spawns`, only a `type`) stays as it is.
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Keys a modifier may carry besides its own, kept as they are.
const KEPT: [&str; 2] = ["neoforge:conditions", "forge:conditions"];

/// Biomes Aquaculture never adds fish to (`BiomeTagPredicate.INVALID_TYPES`).
const NO_FISH: [&str; 3] = ["#minecraft:is_nether", "#minecraft:is_end", "#c:is_void"];

/// Why a modifier of a known type can't be written exactly, so it stays outside.
pub fn inexact(v: &Value) -> Option<String> {
    match v.get("type")?.as_str()? {
        // Its code also matches the biome's name ("warm", "frozen"...) and
        // namespace, which no selector reads.
        "repurposed_structures:additions_temperature_modifier" => Some(
            "its type picks biomes by their names and temperature in the mod's Java, which no \
             biome selector says"
                .into(),
        ),
        _ => None,
    }
}

/// `v` rewritten as NeoForge's modifier when its type is one known here, with the
/// type it had and any further modifiers the one becomes (written beside it). A
/// field that only switches the mod's config, with the default on, goes to `lost`
/// (`<type>: <field>`).
pub fn translate(
    v: &mut Value,
    lost: &mut BTreeMap<String, usize>,
) -> Option<(String, Vec<Value>)> {
    let kind = v.get("type")?.as_str()?.to_owned();
    let (ns, path) = kind.split_once(':')?;
    let mut lose = |field: &str| {
        if v.get(field).is_some() {
            *lost.entry(format!("{kind}: {field}")).or_default() += 1;
        }
    };
    let new: Vec<Value> = match (ns, path) {
        // One ore feature, in the step each modifier's code names.
        ("mysticalagriculture", "inferium_ore" | "prosperity_ore" | "soulstone")
        | (
            "mysticalagradditions",
            "nether_inferium_ore"
            | "nether_prosperity_ore"
            | "end_inferium_ore"
            | "end_prosperity_ore",
        ) => {
            let step = if path == "soulstone" {
                "raw_generation"
            } else {
                "underground_ores"
            };
            vec![add_features(
                v.get("biomes")?.clone(),
                v.get("feature")?,
                step,
            )]
        }
        // Extreme Reactors: its code also requires the End for anglesite and
        // the Nether for benitoite.
        ("bigreactors", "oregen_yellorite" | "oregen_anglesite" | "oregen_benitoite") => {
            let biomes = v.get("biomes")?.clone();
            let biomes = match path {
                "oregen_anglesite" => all(vec![json!("#minecraft:is_end"), biomes]),
                "oregen_benitoite" => all(vec![json!("#minecraft:is_nether"), biomes]),
                _ => biomes,
            };
            vec![add_features(biomes, v.get("feature")?, "underground_ores")]
        }
        ("actuallyadditions", "bool_config_feature_modifier") => {
            lose("boolConfig");
            vec![add_features(
                v.get("biomes")?.clone(),
                v.get("features")?,
                v.get("step")?.as_str()?,
            )]
        }
        ("repurposed_structures", "additions_modifier") => vec![add_features(
            v.get("biomes")?.clone(),
            v.get("feature")?,
            v.get("step")?.as_str()?,
        )],
        ("repurposed_structures", "removals_modifier") => vec![json!({
            "type": "neoforge:remove_features",
            "biomes": v.get("biomes")?,
            "features": v.get("feature")?,
            "steps": v.get("step")?,
        })],
        // A nest where the biome is colder than `maxTemperature`, strictly.
        ("productivebees", "nest") => {
            let below = not(climate(Some(v.get("maxTemperature")?.as_f64()?), None));
            let step = v.get("generation_stage")?.as_str()?.to_ascii_lowercase();
            let biomes = all(vec![v.get("biomes")?.clone(), below]);
            vec![add_features(biomes, v.get("features")?, &step)]
        }
        // The allowed biomes less the denied, within the base temperatures
        // given (both bounds included).
        ("farmersdelight", "add_features_by_filter") => {
            let mut sets = vec![v.get("allowed_biomes")?.clone()];
            if let Some(denied) = v.get("denied_biomes").filter(|d| !is_empty(d)) {
                sets.push(not(any(listed(denied))));
            }
            // A bound that is there but no number: left alone.
            let bound = |b: Option<&Value>| match b {
                None => Some(None),
                Some(x) => x.as_f64().map(Some),
            };
            let min = bound(v.get("min_temperature"))?;
            let max = bound(v.get("max_temperature"))?;
            if min.is_some() || max.is_some() {
                sets.push(climate(min, max));
            }
            vec![add_features(
                all(sets),
                v.get("features")?,
                v.get("step")?.as_str()?,
            )]
        }
        ("aquaculture", "mob_spawn") => {
            let mut sets = vec![v.get("includeBiomes")?.clone()];
            if let Some(excluded) = v.get("excludeBiomes").filter(|e| !is_empty(e)) {
                sets.push(not(any(listed(excluded))));
            }
            vec![add_spawns(all(sets), v.get("spawn")?)]
        }
        ("aquaculture", "fish_spawn") => fish(v)?,
        _ => return None,
    };
    let mut new = new.into_iter().map(|n| match n {
        Value::Object(o) => Some(o),
        _ => None,
    });
    let mut first = new.next()??;
    let mut more = Vec::new();
    for other in new {
        let mut other = other?;
        keep(v, &mut other);
        more.push(Value::Object(other));
    }
    keep(v, &mut first);
    *v = Value::Object(first);
    Some((kind, more))
}

/// Aquaculture's fish (`FishSpawnBiomeModifier`), never in the Nether, the End or the void:
/// with the empty first include (`#aquaculture:empty`) and some excludes, every biome
/// but those; with `and`, the biomes in every include, excludes left aside; otherwise the
/// spawn once for each include the biome is in, unless an exclude holds it.
fn fish(v: &Value) -> Option<Vec<Value>> {
    let include = listed(v.get("includeBiomes")?);
    let exclude = listed(v.get("excludeBiomes")?);
    let spawn = v.get("spawn")?;
    let valid = not(any(NO_FISH.iter().map(|t| json!(t)).collect()));
    let first = include.first()?;
    let mut and = vec![valid.clone()];
    if first == "#aquaculture:empty" && !exclude.is_empty() {
        and.push(not(any(exclude)));
        return Some(vec![add_spawns(all(and), spawn)]);
    }
    if v.get("and").and_then(Value::as_bool).unwrap_or(false) {
        and.extend(include);
        return Some(vec![add_spawns(all(and), spawn)]);
    }
    let excluded = (!exclude.is_empty()).then(|| not(any(exclude)));
    let each = include.into_iter().map(|set| {
        let mut sets = vec![valid.clone(), set];
        sets.extend(excluded.clone());
        add_spawns(all(sets), spawn)
    });
    Some(each.collect())
}

fn add_features(biomes: Value, features: &Value, step: &str) -> Value {
    json!({"type": "neoforge:add_features", "biomes": biomes, "features": features,
        "step": step})
}

fn add_spawns(biomes: Value, spawners: &Value) -> Value {
    json!({"type": "neoforge:add_spawns", "biomes": biomes, "spawners": spawners})
}

/// A biome set as its entries: a list's, or the one id or tag.
fn listed(set: &Value) -> Vec<Value> {
    match set {
        Value::Array(list) => list.clone(),
        other => vec![other.clone()],
    }
}

fn is_empty(set: &Value) -> bool {
    set.is_null() || set.as_array().is_some_and(Vec::is_empty)
}

/// NeoForge's selector of the biomes in all of `sets`; a list may not hold
/// tags, so each is a selector of its own.
fn all(sets: Vec<Value>) -> Value {
    combine("neoforge:and", sets)
}

/// The biomes in any of `sets`.
fn any(sets: Vec<Value>) -> Value {
    combine("neoforge:or", sets)
}

fn combine(kind: &str, mut sets: Vec<Value>) -> Value {
    if sets.len() == 1 {
        return sets.remove(0);
    }
    json!({"type": kind, "values": sets})
}

fn not(set: Value) -> Value {
    json!({"type": "neoforge:not", "value": set})
}

/// Horizon's selector of the biomes whose base temperature lies within
/// `min..=max`.
fn climate(min: Option<f64>, max: Option<f64>) -> Value {
    let mut range = serde_json::Map::new();
    if let Some(min) = min {
        range.insert("min".into(), json!(min));
    }
    if let Some(max) = max {
        range.insert("max".into(), json!(max));
    }
    json!({"type": "horizon:climate", "temperature": range})
}

/// `from`'s conditions on `to`.
fn keep(from: &Value, to: &mut serde_json::Map<String, Value>) {
    for key in KEPT {
        if let Some(x) = from.get(key) {
            to.insert(key.to_owned(), x.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(mut v: Value) -> Value {
        let (_, more) = translate(&mut v, &mut BTreeMap::new()).unwrap();
        assert!(more.is_empty());
        v
    }

    #[test]
    fn mods_modifiers_become_neoforges() {
        let ore = one(json!({"type": "mysticalagriculture:inferium_ore",
            "biomes": "#is_overworld", "feature": "mysticalagriculture:inferium_ore"}));
        assert_eq!(
            ore,
            json!({"type": "neoforge:add_features", "biomes": "#is_overworld",
                "features": "mysticalagriculture:inferium_ore", "step": "underground_ores"})
        );
        let soul = one(json!({"type": "mysticalagriculture:soulstone",
            "biomes": "#is_nether", "feature": "mysticalagriculture:soulstone"}));
        assert_eq!(soul["step"], "raw_generation");
        let anglesite = one(json!({"type": "bigreactors:oregen_anglesite",
            "biomes": "#c:is_end", "feature": "bigreactors:anglesite_ore"}));
        assert_eq!(
            anglesite["biomes"],
            json!({"type": "neoforge:and", "values": ["#minecraft:is_end", "#c:is_end"]})
        );
        let mut lost = BTreeMap::new();
        let mut quartz = json!({"type": "actuallyadditions:bool_config_feature_modifier",
            "biomes": "#minecraft:is_overworld", "boolConfig": "generateQuartz",
            "features": "actuallyadditions:ore_black_quartz", "step": "underground_ores"});
        translate(&mut quartz, &mut lost).unwrap();
        assert_eq!(quartz["type"], "neoforge:add_features");
        assert_eq!(quartz.get("boolConfig"), None);
        assert_eq!(
            lost.keys().collect::<Vec<_>>(),
            ["actuallyadditions:bool_config_feature_modifier: boolConfig"]
        );
        let removal = one(json!({"type": "repurposed_structures:removals_modifier",
            "biomes": "#x:deep", "feature": "minecraft:monster_room_deep",
            "step": "underground_structures"}));
        assert_eq!(
            removal,
            json!({"type": "neoforge:remove_features", "biomes": "#x:deep",
                "features": "minecraft:monster_room_deep", "steps": "underground_structures"})
        );
        // Biomes and features in the mod's Java: left alone.
        let mut code = json!({"type": "cataclysm:cataclysm_mob_spawns"});
        assert_eq!(translate(&mut code, &mut lost), None);
        assert_eq!(code, json!({"type": "cataclysm:cataclysm_mob_spawns"}));
    }

    /// Temperature bounds become `horizon:climate`: Farmer's Delight's both
    /// included, Productive Bees' nest only below its maximum.
    #[test]
    fn temperature_bounds_become_climate_sets() {
        let carrots = one(json!({"type": "farmersdelight:add_features_by_filter",
            "allowed_biomes": "#minecraft:is_overworld",
            "denied_biomes": ["minecraft:lush_caves", "minecraft:mushroom_fields"],
            "features": "farmersdelight:patch_wild_carrots", "step": "vegetal_decoration",
            "min_temperature": 0.4, "max_temperature": 0.9}));
        assert_eq!(
            carrots["biomes"],
            json!({"type": "neoforge:and", "values": ["#minecraft:is_overworld",
                {"type": "neoforge:not", "value": {"type": "neoforge:or",
                    "values": ["minecraft:lush_caves", "minecraft:mushroom_fields"]}},
                {"type": "horizon:climate", "temperature": {"min": 0.4, "max": 0.9}}]})
        );
        let nest = one(
            json!({"type": "productivebees:nest", "biomes": "#c:is_snowy",
            "generation_stage": "TOP_LAYER_MODIFICATION",
            "features": ["productivebees:snow_nest"], "maxTemperature": 0}),
        );
        assert_eq!(nest["step"], "top_layer_modification");
        assert_eq!(
            nest["biomes"],
            json!({"type": "neoforge:and", "values": ["#c:is_snowy",
                {"type": "neoforge:not", "value":
                    {"type": "horizon:climate", "temperature": {"min": 0.0}}}]})
        );
    }

    /// Aquaculture's three ways to pick a fish's biomes, and never the Nether,
    /// the End or the void.
    #[test]
    fn aquaculture_fish_keep_their_include_and_exclude_rules() {
        let valid = json!({"type": "neoforge:not", "value": {"type": "neoforge:or",
            "values": ["#minecraft:is_nether", "#minecraft:is_end", "#c:is_void"]}});
        let spawn = json!({"type": "aquaculture:brown_trout", "weight": 6, "minCount": 1,
            "maxCount": 2});
        // Exclude only: every biome but those.
        let trout = one(json!({"type": "aquaculture:fish_spawn", "spawn": spawn,
            "includeBiomes": ["#aquaculture:empty"], "and": false,
            "excludeBiomes": ["#c:is_ocean", "#c:is_beach"],
            "neoforge:conditions": [{"type": "neoforge:true"}]}));
        assert_eq!(
            trout,
            json!({"type": "neoforge:add_spawns", "spawners": spawn,
                "biomes": {"type": "neoforge:and", "values": [valid,
                    {"type": "neoforge:not", "value": {"type": "neoforge:or",
                        "values": ["#c:is_ocean", "#c:is_beach"]}}]},
                "neoforge:conditions": [{"type": "neoforge:true"}]})
        );
        // `and`: every include, the excludes left aside as Aquaculture does.
        let cold = one(json!({"type": "aquaculture:fish_spawn", "spawn": spawn,
            "includeBiomes": ["#c:is_cold", "#c:is_ocean"], "and": true,
            "excludeBiomes": ["#c:is_deep"]}));
        assert_eq!(
            cold["biomes"],
            json!({"type": "neoforge:and", "values": [valid, "#c:is_cold", "#c:is_ocean"]})
        );
        // Otherwise once per include, so a biome in both gets the fish twice.
        let mut sea = json!({"type": "aquaculture:fish_spawn", "spawn": spawn,
            "includeBiomes": ["#c:is_ocean", "#c:is_beach"], "and": false,
            "excludeBiomes": ["#c:is_cold"]});
        let (_, more) = translate(&mut sea, &mut BTreeMap::new()).unwrap();
        let not_cold = json!({"type": "neoforge:not", "value": "#c:is_cold"});
        assert_eq!(
            sea["biomes"],
            json!({"type": "neoforge:and", "values": [valid, "#c:is_ocean", not_cold]})
        );
        assert_eq!(more.len(), 1);
        assert_eq!(
            more[0]["biomes"],
            json!({"type": "neoforge:and", "values": [valid, "#c:is_beach", not_cold]})
        );
    }

    #[test]
    fn a_modifier_naming_biomes_by_name_is_inexact() {
        let ocean = json!({"type": "repurposed_structures:additions_temperature_modifier",
            "biomes": "#x:ocean", "feature": "x:dungeon", "step": "underground_structures",
            "biome_temperature_allowed": "COLD"});
        assert!(inexact(&ocean).is_some());
        assert_eq!(
            inexact(&json!({"type": "repurposed_structures:additions_modifier"})),
            None
        );
    }
}
