//! Lithostitched: its jigsaw structure, its worldgen modifiers and its
//! processors.
use super::ctov;
use crate::processors::{ProcessorData, carry_all, processors, referenced};
use crate::worldgen::{ENHANCED_TERRAIN, Upgrade, start_checks};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A `structure` file of Lithostitched's jigsaw type as vanilla's, with the
/// start rules it shares with the other libraries' jigsaws (`worldgen`'s
/// `start_checks`). `None` for any other file.
pub(crate) fn upgrade(
    kind: &str,
    v: &mut Value,
    lost: &mut BTreeMap<String, usize>,
) -> Option<bool> {
    if kind != "structure" || v["type"] != "lithostitched:jigsaw" {
        return None;
    }
    start_checks(v, "lowest", lost, false);
    if v.as_object_mut()
        .and_then(|o| o.remove(ENHANCED_TERRAIN))
        .is_some()
    {
        *lost.entry(ENHANCED_TERRAIN.into()).or_default() += 1;
    }
    v["type"] = "minecraft:jigsaw".into();
    Some(true)
}

/// A lithostitched worldgen modifier
/// (`data/<ns>/lithostitched/worldgen_modifier/`) that adds to structure
/// sets or template pools, as a file the game merges into them (the key
/// `horizon:add_to`, `horizon_client::mod_content`): (kind, file). The
/// modifier's conditions are checked against `loaded`, the mods ported
/// along: a compat addition for a mod that is not would place that mod's
/// blocks, which the game does not have. Other modifier types are left out.
/// Both are counted in `report`.
pub(crate) fn modifier(
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

/// The data form of one of Lithostitched's processors: the processors to put
/// in its place, or `None` when it has none (or is not Lithostitched's).
pub(crate) fn processor(
    kind: &str,
    p: &Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> Option<Vec<Value>> {
    match kind {
        "lithostitched:apply_random" => apply_random(p, data, carried).map(|p| vec![p]),
        "lithostitched:condition" => condition(p, data, carried).map(|p| vec![p]),
        "lithostitched:block_swap" => {
            // The same map; the game's swap keeps the properties both have.
            p["blocks"].as_object()?;
            Some(vec![
                json!({"processor_type": "horizon:block_swap", "blocks": p["blocks"]}),
            ])
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
            Some(vec![out])
        }
        _ => None,
    }
}

/// The references (Lithostitched's, and CTOV's compat processors) in a
/// processor list written in; whether any was.
pub(crate) fn expand_references(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    references(list, data, carried, 0)
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
        if let Some(inner) = ctov::modular_compat(&p, data, carried, depth) {
            changed = true;
            out.extend(references(inner, data, carried, depth + 1).0);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_structure_mods::data;
    use crate::processors::carry;

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
}
