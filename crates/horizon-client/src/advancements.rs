//! Advancements: the tree loaded from `data/*/advancement`, the player's
//! progress (`PlayerAdvancements`) and the predicates criteria test
//! (`EntityPredicate`, `ItemPredicate`, `LocationPredicate`, the loot
//! conditions advancements use).
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Task,
    Goal,
    Challenge,
}
#[derive(Clone, Debug)]
pub struct Display {
    pub icon: String,
    pub title: Value,
    pub description: Value,
    pub frame: Frame,
    pub background: Option<String>,
    pub show_toast: bool,
    pub announce: bool,
    pub hidden: bool,
}
#[derive(Clone, Debug)]
pub struct Criterion {
    pub name: String,
    pub trigger: String,
    pub conditions: Value,
}
#[derive(Clone, Debug, Default)]
pub struct Rewards {
    pub recipes: Vec<String>,
    pub experience: i32,
    pub loot: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Advancement {
    pub id: String,
    pub parent: Option<String>,
    pub display: Option<Display>,
    pub criteria: Vec<Criterion>,
    pub requirements: Vec<Vec<String>>,
    pub rewards: Rewards,
}
/// Every advancement, indexed by id and by trigger.
#[derive(Default)]
pub struct Tree {
    pub list: Vec<Advancement>,
    pub by_id: HashMap<String, usize>,
    /// trigger id -> (advancement index, criterion index).
    pub by_trigger: HashMap<String, Vec<(usize, usize)>>,
}
fn frame(v: &Value) -> Frame {
    match v.as_str() {
        Some("goal") => Frame::Goal,
        Some("challenge") => Frame::Challenge,
        _ => Frame::Task,
    }
}
impl Tree {
    /// Loads `<data>/<namespace>/advancement/**/*.json`.
    pub fn load(data: &Path) -> Tree {
        let mut tree = Tree::default();
        let Ok(namespaces) = std::fs::read_dir(data) else {
            return tree;
        };
        let mut files = Vec::new();
        for ns in namespaces.flatten() {
            let root = ns.path().join("advancement");
            collect(&root, &root, &ns.file_name().to_string_lossy(), &mut files);
        }
        files.sort();
        for (id, path) in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(a) = parse(&id, &v) {
                let index = tree.list.len();
                for (ci, c) in a.criteria.iter().enumerate() {
                    tree.by_trigger
                        .entry(c.trigger.clone())
                        .or_default()
                        .push((index, ci));
                }
                tree.by_id.insert(id, index);
                tree.list.push(a);
            }
        }
        tree
    }
    pub fn get(&self, id: &str) -> Option<&Advancement> {
        self.by_id.get(id).map(|&i| &self.list[i])
    }
}
fn collect(root: &Path, dir: &Path, ns: &str, out: &mut Vec<(String, std::path::PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(root, &p, ns, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            let rel = p.strip_prefix(root).unwrap_or(&p).with_extension("");
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.push((format!("{ns}:{rel}"), p));
        }
    }
}
fn parse(id: &str, v: &Value) -> Option<Advancement> {
    let criteria: Vec<Criterion> = v["criteria"]
        .as_object()?
        .iter()
        .map(|(name, c)| Criterion {
            name: name.clone(),
            trigger: c["trigger"].as_str().unwrap_or("").to_owned(),
            conditions: c["conditions"].clone(),
        })
        .collect();
    let requirements = match v["requirements"].as_array() {
        Some(r) => r
            .iter()
            .map(|group| {
                group
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .collect(),
        // AdvancementRequirements.allOf: every criterion.
        None => criteria.iter().map(|c| vec![c.name.clone()]).collect(),
    };
    let d = &v["display"];
    let display = d.is_object().then(|| Display {
        icon: d["icon"]["id"]
            .as_str()
            .unwrap_or("minecraft:air")
            .to_owned(),
        title: d["title"].clone(),
        description: d["description"].clone(),
        frame: frame(&d["frame"]),
        background: d["background"].as_str().map(str::to_owned),
        show_toast: d["show_toast"].as_bool().unwrap_or(true),
        announce: d["announce_to_chat"].as_bool().unwrap_or(true),
        hidden: d["hidden"].as_bool().unwrap_or(false),
    });
    let r = &v["rewards"];
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    Some(Advancement {
        id: id.to_owned(),
        parent: v["parent"].as_str().map(str::to_owned),
        display,
        criteria,
        requirements,
        rewards: Rewards {
            recipes: strings(&r["recipes"]),
            experience: r["experience"].as_i64().unwrap_or(0) as i32,
            loot: strings(&r["loot"]),
        },
    })
}

/// The player's progress: advancement -> criterion -> obtained date.
#[derive(Default, Clone)]
pub struct Progress {
    pub criteria: BTreeMap<String, BTreeMap<String, String>>,
}
impl Progress {
    pub fn has(&self, adv: &str, criterion: &str) -> bool {
        self.criteria
            .get(adv)
            .is_some_and(|m| m.contains_key(criterion))
    }
    /// `AdvancementProgress.isDone`.
    pub fn done(&self, a: &Advancement) -> bool {
        !a.requirements.is_empty()
            && a.requirements
                .iter()
                .all(|group| group.iter().any(|c| self.has(&a.id, c)))
    }
    /// Completed requirement groups and their count (the screen's tooltip).
    pub fn counts(&self, a: &Advancement) -> (usize, usize) {
        let done = a
            .requirements
            .iter()
            .filter(|g| g.iter().any(|c| self.has(&a.id, c)))
            .count();
        (done, a.requirements.len())
    }
    pub fn grant(&mut self, adv: &str, criterion: &str, date: &str) -> bool {
        let m = self.criteria.entry(adv.to_owned()).or_default();
        if m.contains_key(criterion) {
            return false;
        }
        m.insert(criterion.to_owned(), date.to_owned());
        true
    }
    pub fn revoke(&mut self, adv: &str, criterion: &str) -> bool {
        self.criteria
            .get_mut(adv)
            .is_some_and(|m| m.remove(criterion).is_some())
    }
    pub fn from_json(v: &Value) -> Progress {
        let mut p = Progress::default();
        if let Some(o) = v.as_object() {
            for (adv, entry) in o {
                if adv == "DataVersion" {
                    continue;
                }
                if let Some(c) = entry["criteria"].as_object() {
                    let m = p.criteria.entry(adv.clone()).or_default();
                    for (k, d) in c {
                        m.insert(k.clone(), d.as_str().unwrap_or("").to_owned());
                    }
                }
            }
        }
        p
    }
    pub fn to_json(&self, tree: &Tree, data_version: i32) -> Value {
        let mut o = serde_json::Map::new();
        for (adv, m) in &self.criteria {
            if m.is_empty() {
                continue;
            }
            let done = tree.get(adv).is_some_and(|a| self.done(a));
            o.insert(
                adv.clone(),
                serde_json::json!({ "criteria": m, "done": done }),
            );
        }
        o.insert("DataVersion".into(), data_version.into());
        Value::Object(o)
    }
}

/// What predicates may ask about the world.
pub trait World {
    fn dimension(&self) -> &str;
    fn biome(&self, pos: [i32; 3]) -> String;
    fn structures(&self, pos: [i32; 3]) -> Vec<String>;
    /// Block name and properties.
    fn block(&self, pos: [i32; 3]) -> (String, BTreeMap<String, String>);
    fn fluid(&self, pos: [i32; 3]) -> Option<String>;
    fn can_see_sky(&self, pos: [i32; 3]) -> bool;
    fn light(&self, pos: [i32; 3]) -> i32;
    fn smokey(&self, pos: [i32; 3]) -> bool;
    fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool;
}
/// An item stack as predicates see it.
#[derive(Clone, Debug, Default)]
pub struct ItemView {
    pub id: String,
    pub count: u32,
    pub damage: u32,
    pub enchantments: Vec<(String, u32)>,
    pub potion: Option<String>,
    pub custom_name: bool,
}
/// An entity as predicates see it.
#[derive(Clone, Debug, Default)]
pub struct EntityView {
    pub kind: String,
    pub position: [f64; 3],
    pub on_fire: bool,
    pub sneaking: bool,
    pub sprinting: bool,
    pub swimming: bool,
    pub flying: bool,
    pub baby: bool,
    pub on_ground: bool,
    /// mainhand, offhand, head, chest, legs, feet, body.
    pub equipment: [Option<ItemView>; 7],
    pub vehicle: Option<Box<EntityView>>,
    pub effects: Vec<(String, i32, i32)>,
    /// Player: game mode name.
    pub gamemode: Option<String>,
    /// Lightning: entities it struck.
    pub struck: Vec<EntityView>,
    pub fishing_open_water: Option<bool>,
    pub variant: Option<String>,
}
fn range_f(v: &Value, x: f64) -> bool {
    match v {
        Value::Null => true,
        Value::Number(n) => n.as_f64().is_some_and(|n| (n - x).abs() < 1e-9),
        Value::Object(o) => {
            o.get("min").and_then(Value::as_f64).is_none_or(|m| x >= m)
                && o.get("max").and_then(Value::as_f64).is_none_or(|m| x <= m)
        }
        _ => false,
    }
}
fn range_i(v: &Value, x: i64) -> bool {
    range_f(v, x as f64)
}
/// A registry id, `#tag`, or list matched against one id.
fn id_matches(v: &Value, registry: &str, id: &str, w: &dyn World) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => match s.strip_prefix('#') {
            Some(tag) => w.in_tag(registry, tag, id),
            None => s == id,
        },
        Value::Array(a) => a.iter().any(|x| id_matches(x, registry, id, w)),
        _ => false,
    }
}
/// `ItemPredicate`.
pub fn item(v: &Value, it: &ItemView, w: &dyn World) -> bool {
    if v.is_null() {
        return true;
    }
    if !id_matches(&v["items"], "item", &it.id, w) {
        return false;
    }
    if !range_i(&v["count"], i64::from(it.count)) {
        return false;
    }
    // Data components: exact matches on the few the data uses.
    let comps = &v["components"];
    if let Some(p) = comps["minecraft:potion_contents"].as_str()
        && it.potion.as_deref() != Some(p)
    {
        return false;
    }
    let preds = &v["predicates"];
    if let Some(list) = preds["minecraft:enchantments"].as_array() {
        for e in list {
            let ok = it.enchantments.iter().any(|(id, lvl)| {
                id_matches(&e["enchantments"], "enchantment", id, w)
                    && range_i(&e["levels"], i64::from(*lvl))
            });
            if !ok {
                return false;
            }
        }
    }
    if let Some(d) = preds["minecraft:damage"].as_object() {
        let damage = Value::Object(d.clone());
        if !range_i(&damage["damage"], i64::from(it.damage)) {
            return false;
        }
    }
    true
}
/// `BlockPredicate`.
pub fn block(v: &Value, pos: [i32; 3], w: &dyn World) -> bool {
    if v.is_null() {
        return true;
    }
    let (name, props) = w.block(pos);
    if !id_matches(&v["blocks"], "block", &name, w) {
        return false;
    }
    if let Some(state) = v["state"].as_object() {
        for (k, want) in state {
            let have = props.get(k).cloned().unwrap_or_default();
            let ok = match want {
                Value::String(s) => have == *s,
                Value::Bool(b) => have == b.to_string(),
                Value::Number(n) => have == n.to_string(),
                Value::Object(_) => have.parse::<f64>().is_ok_and(|h| range_f(want, h)),
                _ => false,
            };
            if !ok {
                return false;
            }
        }
    }
    true
}
/// `LocationPredicate`.
pub fn location(v: &Value, p: [f64; 3], w: &dyn World) -> bool {
    if v.is_null() {
        return true;
    }
    let pos = [
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32,
    ];
    let posv = &v["position"];
    if !range_f(&posv["x"], p[0]) || !range_f(&posv["y"], p[1]) || !range_f(&posv["z"], p[2]) {
        return false;
    }
    if let Some(d) = v["dimension"].as_str()
        && d != w.dimension()
    {
        return false;
    }
    if !v["biomes"].is_null() && !id_matches(&v["biomes"], "worldgen/biome", &w.biome(pos), w) {
        return false;
    }
    if !v["structures"].is_null() {
        let here = w.structures(pos);
        if !here
            .iter()
            .any(|s| id_matches(&v["structures"], "worldgen/structure", s, w))
        {
            return false;
        }
    }
    if let Some(b) = v["can_see_sky"].as_bool()
        && b != w.can_see_sky(pos)
    {
        return false;
    }
    if let Some(b) = v["smokey"].as_bool()
        && b != w.smokey(pos)
    {
        return false;
    }
    if !v["light"].is_null() && !range_i(&v["light"]["light"], i64::from(w.light(pos))) {
        return false;
    }
    if !v["block"].is_null() && !block(&v["block"], pos, w) {
        return false;
    }
    if !v["fluid"].is_null() {
        let Some(f) = w.fluid(pos) else {
            return false;
        };
        if !id_matches(&v["fluid"]["fluids"], "fluid", &f, w) {
            return false;
        }
    }
    true
}
/// `EntityPredicate` (26.x keyed sub-predicates).
pub fn entity(v: &Value, e: &EntityView, w: &dyn World, origin: [f64; 3]) -> bool {
    if v.is_null() {
        return true;
    }
    let Some(o) = v.as_object() else {
        return false;
    };
    for (key, p) in o {
        let ok = match key.as_str() {
            "minecraft:entity_type" | "type" => id_matches(p, "entity_type", &e.kind, w),
            "minecraft:location" | "location" => location(p, e.position, w),
            "minecraft:stepping_on" => {
                location(p, [e.position[0], e.position[1] - 0.5, e.position[2]], w)
            }
            "minecraft:distance" | "distance" => {
                let d = [
                    e.position[0] - origin[0],
                    e.position[1] - origin[1],
                    e.position[2] - origin[2],
                ];
                range_f(&p["x"], d[0].abs())
                    && range_f(&p["y"], d[1].abs())
                    && range_f(&p["z"], d[2].abs())
                    && range_f(&p["horizontal"], (d[0] * d[0] + d[2] * d[2]).sqrt())
                    && range_f(
                        &p["absolute"],
                        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt(),
                    )
            }
            "minecraft:flags" | "flags" => {
                let f = |k: &str, have: bool| p[k].as_bool().is_none_or(|want| want == have);
                f("is_on_fire", e.on_fire)
                    && f("is_sneaking", e.sneaking)
                    && f("is_sprinting", e.sprinting)
                    && f("is_swimming", e.swimming)
                    && f("is_flying", e.flying)
                    && f("is_baby", e.baby)
                    && f("is_on_ground", e.on_ground)
            }
            "minecraft:equipment" | "equipment" => {
                let slots = [
                    "mainhand", "offhand", "head", "chest", "legs", "feet", "body",
                ];
                slots.iter().enumerate().all(|(i, s)| {
                    p[*s].is_null()
                        || e.equipment[i]
                            .as_ref()
                            .is_some_and(|it| item(&p[*s], it, w))
                })
            }
            "minecraft:vehicle" | "vehicle" => e
                .vehicle
                .as_ref()
                .is_some_and(|veh| entity(p, veh, w, origin)),
            "minecraft:effects" | "effects" => p.as_object().is_some_and(|m| {
                m.iter().all(|(id, want)| {
                    e.effects.iter().any(|(eid, amp, dur)| {
                        eid == id
                            && range_i(&want["amplifier"], i64::from(*amp))
                            && range_i(&want["duration"], i64::from(*dur))
                    })
                })
            }),
            "minecraft:type_specific/player" => p["gamemode"].as_array().is_none_or(|modes| {
                e.gamemode
                    .as_deref()
                    .is_some_and(|g| modes.iter().any(|m| m.as_str() == Some(g)))
            }),
            "minecraft:type_specific/lightning" => {
                range_i(&p["blocks_set_on_fire"], 0)
                    && (p["entity_struck"].is_null()
                        || e.struck
                            .iter()
                            .any(|s| entity(&p["entity_struck"], s, w, origin)))
            }
            "minecraft:type_specific/fishing_hook" => p["in_open_water"]
                .as_bool()
                .is_none_or(|want| e.fishing_open_water == Some(want)),
            "minecraft:components" | "minecraft:predicates" => true,
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}
/// The entities a loot-style condition list may refer to.
pub struct Entities<'a> {
    pub this: Option<&'a EntityView>,
    pub origin: [f64; 3],
    pub tool: Option<&'a ItemView>,
    pub block_pos: Option<[i32; 3]>,
}
/// A `ContextAwarePredicate`: a list of loot conditions (or one condition).
pub fn conditions(v: &Value, ctx: &Entities, w: &dyn World) -> bool {
    match v {
        Value::Null => true,
        Value::Array(list) => list.iter().all(|c| conditions(c, ctx, w)),
        Value::Object(o) if o.contains_key("condition") || o.contains_key("type") => {
            let ty = o
                .get("condition")
                .or_else(|| o.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match ty {
                "minecraft:entity_properties" => ctx
                    .this
                    .is_some_and(|e| entity(&v["predicate"], e, w, ctx.origin)),
                "minecraft:location_check" => {
                    let base = ctx.block_pos.map_or(ctx.origin, |b| {
                        [
                            f64::from(b[0]) + 0.5,
                            f64::from(b[1]) + 0.5,
                            f64::from(b[2]) + 0.5,
                        ]
                    });
                    let off = [
                        v["offsetX"].as_f64().unwrap_or(0.0),
                        v["offsetY"].as_f64().unwrap_or(0.0),
                        v["offsetZ"].as_f64().unwrap_or(0.0),
                    ];
                    location(
                        &v["predicate"],
                        [base[0] + off[0], base[1] + off[1], base[2] + off[2]],
                        w,
                    )
                }
                "minecraft:block_state_property" => ctx.block_pos.is_some_and(|b| {
                    let mut pred = serde_json::Map::new();
                    pred.insert("blocks".into(), v["block"].clone());
                    if !v["properties"].is_null() {
                        pred.insert("state".into(), v["properties"].clone());
                    }
                    block(&Value::Object(pred), b, w)
                }),
                "minecraft:match_tool" => ctx.tool.is_some_and(|t| item(&v["predicate"], t, w)),
                "minecraft:inverted" => !conditions(&v["term"], ctx, w),
                "minecraft:all_of" => v["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|t| conditions(t, ctx, w)),
                "minecraft:any_of" => v["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|t| conditions(t, ctx, w)),
                _ => false,
            }
        }
        // An inline entity predicate (old-style).
        Value::Object(_) => ctx.this.is_some_and(|e| entity(v, e, w, ctx.origin)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Flat;
    impl World for Flat {
        fn dimension(&self) -> &str {
            "minecraft:overworld"
        }
        fn biome(&self, _: [i32; 3]) -> String {
            "minecraft:plains".into()
        }
        fn structures(&self, _: [i32; 3]) -> Vec<String> {
            vec!["minecraft:village_plains".into()]
        }
        fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
            ("minecraft:stone".into(), BTreeMap::new())
        }
        fn fluid(&self, _: [i32; 3]) -> Option<String> {
            None
        }
        fn can_see_sky(&self, _: [i32; 3]) -> bool {
            true
        }
        fn light(&self, _: [i32; 3]) -> i32 {
            15
        }
        fn smokey(&self, _: [i32; 3]) -> bool {
            false
        }
        fn in_tag(&self, _: &str, tag: &str, id: &str) -> bool {
            tag == "minecraft:stone_tool_materials" && id == "minecraft:cobblestone"
        }
    }
    #[test]
    fn predicates_match_tags_ranges_and_locations() {
        let w = Flat;
        let stone = ItemView {
            id: "minecraft:cobblestone".into(),
            count: 3,
            ..Default::default()
        };
        assert!(item(
            &serde_json::json!({"items": "#minecraft:stone_tool_materials"}),
            &stone,
            &w
        ));
        assert!(!item(
            &serde_json::json!({"items": "minecraft:dirt"}),
            &stone,
            &w
        ));
        assert!(item(&serde_json::json!({"count": {"min": 2}}), &stone, &w));
        let player = EntityView {
            kind: "minecraft:player".into(),
            position: [0.5, 64.0, 0.5],
            ..Default::default()
        };
        let cond = serde_json::json!({
            "type": "minecraft:entity_properties",
            "entity": "this",
            "predicate": {"minecraft:location": {"structures": "minecraft:village_plains"}}
        });
        let ctx = Entities {
            this: Some(&player),
            origin: player.position,
            tool: None,
            block_pos: None,
        };
        assert!(conditions(&cond, &ctx, &w));
        let mut p = Progress::default();
        let a = parse(
            "minecraft:t",
            &serde_json::json!({"criteria": {"a": {"trigger": "minecraft:tick"}, "b": {"trigger": "minecraft:tick"}},
                "requirements": [["a", "b"]]}),
        )
        .unwrap();
        assert!(!p.done(&a));
        p.grant("minecraft:t", "b", "now");
        assert!(p.done(&a));
    }
}
