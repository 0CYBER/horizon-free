//! Choice Theorem's Overhauled Village: the entries it adds to other
//! structure sets in Java code, and its `modular_compat` processor.
use crate::processors::{ProcessorData, referenced};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

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
pub(crate) fn code_set_entries(
    mod_id: &str,
) -> Vec<(&'static str, &'static str, Vec<(String, u32)>)> {
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

/// Choice Theorem's Overhauled Village's processor that runs the list
/// `reference` when mod `modid` is loaded (ModularCompatProcessor).
const MODULAR_COMPAT: &str = "ctov:modular_compat";

/// Notes which mods its compat processors name are `loaded`.
pub(crate) fn mark_loaded(data: &mut ProcessorData, loaded: &dyn Fn(&str) -> bool) {
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
    data.lists.values().for_each(|l| walk(l, &mut named));
    data.compat_loaded = named.into_iter().filter(|m| loaded(m)).collect();
}

/// A `ctov:modular_compat` processor in a list being carried, at `depth`
/// (see [`super::lithostitched`]'s references): counted in `carried`, and
/// the processors of the list it names when its mod is loaded, else none.
/// `None` for any other processor, or past the depth limit.
pub(super) fn modular_compat(
    p: &Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
    depth: usize,
) -> Option<Vec<Value>> {
    if p["processor_type"] != MODULAR_COMPAT || depth >= 8 {
        return None;
    }
    *carried.entry(MODULAR_COMPAT.into()).or_default() += 1;
    let loaded = p["modid"]
        .as_str()
        .is_some_and(|m| data.compat_loaded.contains(m));
    Some(
        loaded
            .then(|| referenced(&p["reference"], data))
            .flatten()
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processors::carry;
    use serde_json::json;

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
}
