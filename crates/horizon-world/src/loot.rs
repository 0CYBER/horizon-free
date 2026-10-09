//! Container loot: `LootTable.fill` for chest-style loot tables (pools,
//! weighted entries, nested tables, item modifiers and conditions, the
//! stack splitter and the slot shuffle) with vanilla's random draw order.
//!
//! Items keep their id, count, damage, enchantments and potion. Exploration
//! maps are kept as found (structure search is not run); conditions that
//! need an entity, tool or damage source fail, as they do for containers.
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::Path,
};

/// One generated item stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LootStack {
    pub item: String,
    pub count: i32,
    pub damage: i32,
    pub enchantments: Vec<(String, i32)>,
    pub potion: Option<String>,
    /// Set by exploration_map (the map_id component of a found map).
    pub map: bool,
    /// Set by set_ominous_bottle_amplifier.
    pub ominous_amplifier: Option<i32>,
}
impl LootStack {
    fn new(item: &str) -> Self {
        Self {
            item: item.to_owned(),
            count: 1,
            damage: 0,
            enchantments: Vec::new(),
            potion: None,
            map: false,
            ominous_amplifier: None,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.count <= 0 || self.item == "minecraft:air"
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ItemFacts {
    max_stack: i32,
    max_damage: i32,
    enchantable: Option<i32>,
}
#[derive(Debug)]
struct Enchantment {
    name: String,
    supported: Value,
    primary: Option<Value>,
    exclusive: Value,
    max_level: i32,
    weight: i32,
    min_cost: (i32, i32),
    max_cost: (i32, i32),
}
impl Enchantment {
    fn min_cost(&self, level: i32) -> i32 {
        self.min_cost.0 + self.min_cost.1 * (level - 1)
    }
    fn max_cost(&self, level: i32) -> i32 {
        self.max_cost.0 + self.max_cost.1 * (level - 1)
    }
}

/// Loot tables, items, enchantments and the tags they reference.
pub struct LootData {
    tables: HashMap<String, Value>,
    modifiers: HashMap<String, Value>,
    predicates: HashMap<String, Value>,
    items: HashMap<String, ItemFacts>,
    /// Enchantments in registry (identifier) order.
    enchantments: Vec<Enchantment>,
    /// Raw tag files by (registry, tag id).
    tags: HashMap<(String, String), Vec<Value>>,
}

fn load_dir(dir: &Path, prefix: &str, out: &mut HashMap<String, Value>) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.is_dir() {
            load_dir(&path, &format!("{prefix}{stem}/"), out)?;
        } else if path.extension().is_some_and(|e| e == "json") {
            let value = serde_json::from_slice(&fs::read(&path)?)?;
            out.insert(format!("minecraft:{prefix}{stem}"), value);
        }
    }
    Ok(())
}
fn cost(v: &Value) -> (i32, i32) {
    (
        v["base"].as_i64().unwrap_or(0) as i32,
        v["per_level_above_first"].as_i64().unwrap_or(0) as i32,
    )
}

impl LootData {
    /// Load from a resource root holding `items.json` and `data/minecraft`.
    pub fn load(root: &Path) -> io::Result<Self> {
        let data = root.join("data/minecraft");
        let mut tables = HashMap::new();
        load_dir(&data.join("loot_table"), "", &mut tables)?;
        let mut modifiers = HashMap::new();
        load_dir(&data.join("item_modifier"), "", &mut modifiers)?;
        let mut predicates = HashMap::new();
        load_dir(&data.join("predicate"), "", &mut predicates)?;
        let raw: BTreeMap<String, Value> =
            serde_json::from_slice(&fs::read(root.join("items.json"))?)?;
        let items = raw
            .iter()
            .map(|(name, c)| {
                (
                    name.clone(),
                    ItemFacts {
                        max_stack: c["minecraft:max_stack_size"].as_i64().unwrap_or(64) as i32,
                        max_damage: c["minecraft:max_damage"].as_i64().unwrap_or(0) as i32,
                        enchantable: c["minecraft:enchantable"]["value"]
                            .as_i64()
                            .map(|v| v as i32),
                    },
                )
            })
            .collect();
        let mut raw_enchantments = HashMap::new();
        load_dir(&data.join("enchantment"), "", &mut raw_enchantments)?;
        let mut names: Vec<&String> = raw_enchantments.keys().collect();
        names.sort();
        let enchantments = names
            .into_iter()
            .map(|name| {
                let e = &raw_enchantments[name];
                Enchantment {
                    name: name.clone(),
                    supported: e["supported_items"].clone(),
                    primary: e.get("primary_items").cloned(),
                    exclusive: e.get("exclusive_set").cloned().unwrap_or(Value::Null),
                    max_level: e["max_level"].as_i64().unwrap_or(1) as i32,
                    weight: e["weight"].as_i64().unwrap_or(1) as i32,
                    min_cost: cost(&e["min_cost"]),
                    max_cost: cost(&e["max_cost"]),
                }
            })
            .collect();
        let mut tags = HashMap::new();
        for registry in ["item", "enchantment", "instrument"] {
            let mut raw = HashMap::new();
            load_dir(&data.join("tags").join(registry), "", &mut raw)?;
            for (name, value) in raw {
                let values = value["values"].as_array().cloned().unwrap_or_default();
                tags.insert((registry.to_owned(), name), values);
            }
        }
        Ok(Self {
            tables,
            modifiers,
            predicates,
            items,
            enchantments,
            tags,
        })
    }
    pub fn has_table(&self, table: &str) -> bool {
        self.tables.contains_key(table)
    }
    /// Tag members in tag order (nested tags expanded in place, duplicates
    /// dropped).
    fn tag(&self, registry: &str, name: &str) -> Vec<String> {
        fn visit(data: &LootData, registry: &str, name: &str, out: &mut Vec<String>, depth: u32) {
            if depth > 16 {
                return;
            }
            let Some(values) = data.tags.get(&(registry.to_owned(), name.to_owned())) else {
                return;
            };
            for v in values {
                let id = match v {
                    Value::String(s) => s.as_str(),
                    Value::Object(o) => o.get("id").and_then(Value::as_str).unwrap_or(""),
                    _ => continue,
                };
                match id.strip_prefix('#') {
                    Some(tag) => visit(data, registry, tag, out, depth + 1),
                    None if !out.iter().any(|x| x == id) => out.push(id.to_owned()),
                    None => {}
                }
            }
        }
        let mut out = Vec::new();
        visit(self, registry, name, &mut out, 0);
        out
    }
    /// A HolderSet: "#tag", "id" or a list of ids.
    fn holder_set(&self, registry: &str, v: &Value) -> Vec<String> {
        match v {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => self.tag(registry, tag),
                None => vec![s.clone()],
            },
            Value::Array(a) => a
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    }
    fn item_in(&self, item: &str, set: &Value) -> bool {
        self.holder_set("item", set).iter().any(|i| i == item)
    }
    fn facts(&self, item: &str) -> ItemFacts {
        self.items.get(item).copied().unwrap_or(ItemFacts {
            max_stack: 64,
            ..ItemFacts::default()
        })
    }

    /// LootTable.getRandomItems with the given random (its state is
    /// advanced), luck and, for fishing, whether the hook is in open water.
    pub fn roll(
        &self,
        table: &str,
        random: &mut LegacyRandom,
        luck: f32,
        biome: Option<&str>,
        open_water: Option<bool>,
    ) -> Vec<LootStack> {
        let Some(json) = self.tables.get(table) else {
            return Vec::new();
        };
        let mut c = Context {
            data: self,
            random: random.clone(),
            biome,
            visited: Vec::new(),
            luck,
            open_water,
        };
        let mut items = Vec::new();
        c.table_raw(table, json, &mut |_, stack| items.push(stack));
        *random = c.random;
        items
    }
    /// LootTable.fill into an empty container of `slots` slots with
    /// `RandomSource.create(seed)`; `biome` is the container's biome for
    /// location checks (None: no origin, checks fail).
    pub fn fill(
        &self,
        table: &str,
        seed: i64,
        slots: usize,
        biome: Option<&str>,
    ) -> Vec<Option<LootStack>> {
        let mut out = vec![None; slots];
        let Some(json) = self.tables.get(table) else {
            return out;
        };
        let mut c = Context {
            data: self,
            random: LegacyRandom::new(seed),
            biome,
            visited: Vec::new(),
            luck: 0.0,
            open_water: None,
        };
        let mut items = Vec::new();
        c.table_raw(table, json, &mut |c, stack| {
            // LootTable.createStackSplitter (every item is enabled).
            let max = c.data.facts(&stack.item).max_stack.max(1);
            if stack.count < max {
                items.push(stack);
            } else {
                let mut count = stack.count;
                while count > 0 {
                    let mut copy = stack.clone();
                    copy.count = max.min(count);
                    count -= copy.count;
                    items.push(copy);
                }
            }
        });
        let random = &mut c.random;
        let mut available: Vec<usize> = (0..slots).collect();
        shuffle(&mut available, random);
        shuffle_and_split(&mut items, available.len(), random);
        for stack in items {
            let Some(slot) = available.pop() else { break };
            out[slot] = (!stack.is_empty()).then_some(stack);
        }
        out
    }
}

/// Util.shuffle.
fn shuffle<T>(list: &mut [T], random: &mut LegacyRandom) {
    for i in (2..=list.len()).rev() {
        let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
        list.swap(i - 1, j);
    }
}
/// Mth.nextInt.
fn next_int(random: &mut LegacyRandom, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        random.next_int_bound(max - min + 1).unwrap_or(0) + min
    }
}
/// LootTable.shuffleAndSplitItems.
fn shuffle_and_split(result: &mut Vec<LootStack>, slots: usize, random: &mut LegacyRandom) {
    let mut splittable = Vec::new();
    result.retain(|s| {
        if s.is_empty() {
            false
        } else if s.count > 1 {
            splittable.push(s.clone());
            false
        } else {
            true
        }
    });
    while slots as i64 - result.len() as i64 - splittable.len() as i64 > 0 && !splittable.is_empty()
    {
        let index = next_int(random, 0, splittable.len() as i32 - 1) as usize;
        let mut stack = splittable.remove(index);
        let remove = next_int(random, 1, stack.count / 2).min(stack.count);
        let mut copy = stack.clone();
        copy.count = remove;
        stack.count -= remove;
        if stack.count > 1 && random.next_boolean() {
            splittable.push(stack);
        } else {
            result.push(stack);
        }
        if copy.count > 1 && random.next_boolean() {
            splittable.push(copy);
        } else {
            result.push(copy);
        }
    }
    result.append(&mut splittable);
    shuffle(result, random);
}

struct Context<'a> {
    data: &'a LootData,
    random: LegacyRandom,
    biome: Option<&'a str>,
    visited: Vec<String>,
    /// LootContext.getLuck.
    luck: f32,
    /// A fishing hook's `is_open_water` for `entity_properties` (None: no entity).
    open_water: Option<bool>,
}
type Sink<'s> = dyn FnMut(&mut Context<'_>, LootStack) + 's;

impl Context<'_> {
    fn int(&mut self, v: &Value) -> i32 {
        match v {
            Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
            Value::Object(_) => match v["type"].as_str().unwrap_or("") {
                "minecraft:constant" => self.int(&v["value"]),
                "minecraft:uniform" => {
                    let min = self.int(&v["min"]);
                    let max = self.int(&v["max"]);
                    next_int(&mut self.random, min, max)
                }
                "minecraft:binomial" => {
                    let n = self.int(&v["n"]);
                    let p = self.float(&v["p"]);
                    (0..n).filter(|_| self.random.next_float() < p).count() as i32
                }
                _ => 0,
            },
            _ => 0,
        }
    }
    fn float(&mut self, v: &Value) -> f32 {
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.) as f32,
            Value::Object(_) => match v["type"].as_str().unwrap_or("") {
                "minecraft:constant" => self.float(&v["value"]),
                "minecraft:uniform" => {
                    let min = self.float(&v["min"]);
                    let max = self.float(&v["max"]);
                    if min >= max {
                        min
                    } else {
                        self.random.next_float() * (max - min) + min
                    }
                }
                _ => 0.,
            },
            _ => 0.,
        }
    }
    fn condition(&mut self, c: &Value) -> bool {
        let data = self.data;
        match c {
            Value::Null => true,
            Value::Array(list) => list.iter().all(|c| self.condition(c)),
            Value::String(id) => match data.predicates.get(id) {
                Some(p) => self.condition(p),
                None => false,
            },
            Value::Object(_) => match c["type"].as_str().unwrap_or("") {
                "minecraft:inverted" => !self.condition(&c["term"]),
                "minecraft:all_of" => c["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|t| self.condition(t)),
                "minecraft:any_of" => c["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|t| self.condition(t)),
                // FishingHookPredicate: the only entity property a hook has.
                "minecraft:entity_properties" => {
                    let fishing = &c["predicate"]["type_specific"];
                    match (self.open_water, fishing["type"].as_str()) {
                        (Some(open), Some("minecraft:fishing_hook")) => fishing["in_open_water"]
                            .as_bool()
                            .is_none_or(|want| want == open),
                        _ => false,
                    }
                }
                "minecraft:random_chance" => {
                    let chance = self.float(&c["chance"]);
                    self.random.next_float() < chance
                }
                "minecraft:location_check" => {
                    let Some(biome) = self.biome else {
                        return false;
                    };
                    match &c["predicate"]["biomes"] {
                        Value::Null => true,
                        Value::String(s) => match s.strip_prefix('#') {
                            // Biome tags are not loaded here.
                            Some(_) => false,
                            None => s == biome,
                        },
                        Value::Array(a) => a.iter().any(|b| b.as_str() == Some(biome)),
                        _ => false,
                    }
                }
                _ => false,
            },
            _ => false,
        }
    }
    /// An item modifier (typed object, inline list or registry reference).
    fn modify(&mut self, m: &Value, stack: LootStack) -> LootStack {
        let data = self.data;
        match m {
            Value::Null => stack,
            Value::Array(list) => list.iter().fold(stack, |s, f| self.modify(f, s)),
            Value::String(id) => match data.modifiers.get(id) {
                Some(f) => self.modify(f, stack),
                None => stack,
            },
            Value::Object(_) => {
                if !m["condition"].is_null() && !self.condition(&m["condition"]) {
                    return stack;
                }
                self.run(m, stack)
            }
            _ => stack,
        }
    }
    fn run(&mut self, f: &Value, mut stack: LootStack) -> LootStack {
        let data = self.data;
        match f["type"].as_str().unwrap_or("") {
            "minecraft:sequence" => {
                for function in f["functions"].as_array().into_iter().flatten() {
                    stack = self.modify(function, stack);
                }
            }
            "minecraft:set_count" => {
                let base = if f["add"].as_bool() == Some(true) {
                    stack.count
                } else {
                    0
                };
                stack.count = base + self.int(&f["count"]);
            }
            "minecraft:limit_count" => {
                let limit = &f["limit"];
                if let Some(min) = limit["min"].as_i64() {
                    stack.count = stack.count.max(min as i32);
                }
                if let Some(max) = limit["max"].as_i64() {
                    stack.count = stack.count.min(max as i32);
                }
            }
            "minecraft:set_damage" => {
                let max = data.facts(&stack.item).max_damage;
                if max > 0 {
                    let base = if f["add"].as_bool() == Some(true) {
                        1. - stack.damage as f32 / max as f32
                    } else {
                        0.
                    };
                    let pct = 1. - (self.float(&f["damage"]) + base).clamp(0., 1.);
                    stack.damage = (pct * max as f32).floor() as i32;
                }
            }
            "minecraft:enchant_randomly" => {
                let book = stack.item == "minecraft:book";
                let check = !book && f["only_compatible"].as_bool().unwrap_or(true);
                let options: Vec<usize> = match f.get("options") {
                    Some(o) => data
                        .holder_set("enchantment", o)
                        .iter()
                        .filter_map(|n| data.enchantments.iter().position(|e| &e.name == n))
                        .collect(),
                    None => (0..data.enchantments.len()).collect(),
                };
                let options: Vec<usize> = options
                    .into_iter()
                    .filter(|&i| {
                        !check || data.item_in(&stack.item, &data.enchantments[i].supported)
                    })
                    .collect();
                if !options.is_empty() {
                    let pick = self
                        .random
                        .next_int_bound(options.len() as i32)
                        .unwrap_or(0) as usize;
                    let e = &data.enchantments[options[pick]];
                    let level = next_int(&mut self.random, 1, e.max_level);
                    if book {
                        stack.item = "minecraft:enchanted_book".into();
                    }
                    enchant(&mut stack, &e.name, level);
                }
            }
            "minecraft:enchant_with_levels" => {
                let cost = self.int(&f["levels"]);
                let source: Vec<usize> = match f.get("options") {
                    Some(o) => data
                        .holder_set("enchantment", o)
                        .iter()
                        .filter_map(|n| data.enchantments.iter().position(|e| &e.name == n))
                        .collect(),
                    None => (0..data.enchantments.len()).collect(),
                };
                let chosen = self.select_enchantments(&stack.item, cost, &source);
                if stack.item == "minecraft:book" {
                    stack.item = "minecraft:enchanted_book".into();
                }
                for (i, level) in chosen {
                    enchant(&mut stack, &data.enchantments[i].name, level);
                }
            }
            "minecraft:set_enchantments" => {
                if let Some(map) = f["enchantments"].as_object() {
                    if stack.item == "minecraft:book" {
                        stack.item = "minecraft:enchanted_book".into();
                    }
                    for (name, level) in map {
                        let level = self.int(level);
                        enchant(&mut stack, name, level);
                    }
                }
            }
            "minecraft:set_potion" => stack.potion = f["id"].as_str().map(str::to_owned),
            "minecraft:set_stew_effect" => {
                let effects = f["effects"].as_array().cloned().unwrap_or_default();
                if stack.item == "minecraft:suspicious_stew" && !effects.is_empty() {
                    let pick = self
                        .random
                        .next_int_bound(effects.len() as i32)
                        .unwrap_or(0) as usize;
                    self.int(&effects[pick]["duration"]);
                }
            }
            "minecraft:set_ominous_bottle_amplifier" => {
                stack.ominous_amplifier = Some(self.int(&f["amplifier"]));
            }
            "minecraft:set_instrument" => {
                let options = data.holder_set("instrument", &f["options"]);
                if !options.is_empty() {
                    self.random
                        .next_int_bound(options.len() as i32)
                        .unwrap_or(0);
                }
            }
            "minecraft:set_item" => {
                if let Some(item) = f["item"].as_str() {
                    stack.item = item.to_owned();
                }
            }
            // ExplorationMapFunction needs an origin; with one, the map is
            // taken as found (the structure search itself is not run).
            "minecraft:exploration_map" => stack.map = self.biome.is_some(),
            "minecraft:filtered" => {
                let pass = self.item_matches(&f["item_filter"], &stack);
                let branch = if pass { &f["on_pass"] } else { &f["on_fail"] };
                stack = self.modify(branch, stack);
            }
            "minecraft:discard" => stack.count = 0,
            // Names, lore, components and other data without random draws.
            _ => {}
        }
        stack
    }
    /// ItemPredicate subset used by filtered modifiers.
    fn item_matches(&self, p: &Value, stack: &LootStack) -> bool {
        if !p["items"].is_null() && !self.data.item_in(&stack.item, &p["items"]) {
            return false;
        }
        if p["predicates"].get("minecraft:map_id").is_some() && !stack.map {
            return false;
        }
        true
    }
    /// EnchantmentHelper.selectEnchantment.
    fn select_enchantments(
        &mut self,
        item: &str,
        mut cost: i32,
        source: &[usize],
    ) -> Vec<(usize, i32)> {
        let data = self.data;
        let mut results = Vec::new();
        let Some(value) = data.facts(item).enchantable else {
            return results;
        };
        cost += 1
            + self.random.next_int_bound(value / 4 + 1).unwrap_or(0)
            + self.random.next_int_bound(value / 4 + 1).unwrap_or(0);
        let span = (self.random.next_float() + self.random.next_float() - 1.) * 0.15;
        cost = ((cost as f32 + cost as f32 * span + 0.5).floor() as i32).max(1);
        let book = item == "minecraft:book";
        let mut available: Vec<(usize, i32)> = Vec::new();
        for &i in source {
            let e = &data.enchantments[i];
            let primary = data.item_in(item, &e.supported)
                && e.primary.as_ref().is_none_or(|p| data.item_in(item, p));
            if !primary && !book {
                continue;
            }
            for level in (1..=e.max_level).rev() {
                if cost >= e.min_cost(level) && cost <= e.max_cost(level) {
                    available.push((i, level));
                    break;
                }
            }
        }
        if available.is_empty() {
            return results;
        }
        let pick = |random: &mut LegacyRandom, list: &[(usize, i32)]| -> Option<(usize, i32)> {
            let total: i32 = list.iter().map(|&(i, _)| data.enchantments[i].weight).sum();
            if total <= 0 {
                return None;
            }
            let mut selection = random.next_int_bound(total).unwrap_or(0);
            for &entry in list {
                selection -= data.enchantments[entry.0].weight;
                if selection < 0 {
                    return Some(entry);
                }
            }
            None
        };
        results.extend(pick(&mut self.random, &available));
        while self.random.next_int_bound(50).unwrap_or(0) <= cost {
            if let Some(&(last, _)) = results.last() {
                available.retain(|&(i, _)| self.compatible(last, i));
            }
            if available.is_empty() {
                break;
            }
            results.extend(pick(&mut self.random, &available));
            cost /= 2;
        }
        results
    }
    /// Enchantment.areCompatible.
    fn compatible(&self, a: usize, b: usize) -> bool {
        let data = self.data;
        let (ea, eb) = (&data.enchantments[a], &data.enchantments[b]);
        a != b
            && !data
                .holder_set("enchantment", &ea.exclusive)
                .contains(&eb.name)
            && !data
                .holder_set("enchantment", &eb.exclusive)
                .contains(&ea.name)
    }
    /// LootTable.getRandomItemsRaw.
    fn table_raw(&mut self, name: &str, table: &Value, sink: &mut Sink<'_>) {
        if self.visited.iter().any(|v| v == name) {
            return;
        }
        self.visited.push(name.to_owned());
        let table_modifier = &table["modifier"];
        for pool in table["pools"].as_array().into_iter().flatten() {
            if !pool["condition"].is_null() && !self.condition(&pool["condition"]) {
                continue;
            }
            let bonus = if pool["bonus_rolls"].is_null() {
                0.0
            } else {
                self.float(&pool["bonus_rolls"])
            };
            let rolls = self.int(&pool["rolls"]) + (bonus * self.luck).floor() as i32;
            for _ in 0..rolls {
                self.add_random_item(pool, &mut |c, stack| {
                    let stack = c.modify(&pool["modifier"], stack);
                    let stack = c.modify(table_modifier, stack);
                    sink(c, stack);
                });
            }
        }
        self.visited.pop();
    }
    /// LootPool.addRandomItem.
    fn add_random_item(&mut self, pool: &Value, sink: &mut Sink<'_>) {
        let mut valid: Vec<&Value> = Vec::new();
        for entry in pool["entries"].as_array().into_iter().flatten() {
            self.expand(entry, &mut valid);
        }
        // EntryBase.getWeight: max(floor(weight + quality * luck), 0).
        let luck = self.luck;
        let weight = |e: &Value| {
            let w = e["weight"].as_i64().unwrap_or(1) as f32
                + e["quality"].as_i64().unwrap_or(0) as f32 * luck;
            w.floor().max(0.0) as i32
        };
        let total: i32 = valid.iter().map(|e| weight(e)).sum();
        if total == 0 || valid.is_empty() {
            return;
        }
        let chosen = if valid.len() == 1 {
            valid[0]
        } else {
            let mut index = self.random.next_int_bound(total).unwrap_or(0);
            let Some(e) = valid.iter().find(|e| {
                index -= weight(e);
                index < 0
            }) else {
                return;
            };
            e
        };
        self.create(chosen, sink);
    }
    /// LootPoolEntryContainer.expand: the singleton entries that can run.
    fn expand<'v>(&mut self, entry: &'v Value, out: &mut Vec<&'v Value>) -> bool {
        if !entry["condition"].is_null() && !self.condition(&entry["condition"]) {
            return false;
        }
        let children = || entry["children"].as_array().into_iter().flatten();
        match entry["type"].as_str().unwrap_or("") {
            "minecraft:alternatives" => children().any(|c| self.expand(c, out)),
            "minecraft:group" => {
                for c in children() {
                    self.expand(c, out);
                }
                true
            }
            "minecraft:sequence" => {
                for c in children() {
                    if !self.expand(c, out) {
                        return false;
                    }
                }
                true
            }
            "minecraft:item" | "minecraft:empty" | "minecraft:loot_table" => {
                if entry["weight"].as_i64().unwrap_or(1) > 0 {
                    out.push(entry);
                }
                true
            }
            _ => false,
        }
    }
    /// LootPoolEntry.createItemStack with the entry's modifier.
    fn create(&mut self, entry: &Value, sink: &mut Sink<'_>) {
        let modifier = &entry["modifier"];
        let mut out = |c: &mut Context<'_>, stack: LootStack| {
            let stack = c.modify(modifier, stack);
            sink(c, stack);
        };
        match entry["type"].as_str().unwrap_or("") {
            "minecraft:item" => {
                let stack = LootStack::new(entry["name"].as_str().unwrap_or("minecraft:air"));
                out(self, stack);
            }
            "minecraft:loot_table" => match &entry["value"] {
                Value::String(name) => {
                    if let Some(table) = self.data.tables.get(name) {
                        self.table_raw(name, table, &mut out);
                    }
                }
                inline @ Value::Object(_) => self.table_raw("<inline>", inline, &mut out),
                _ => {}
            },
            _ => {}
        }
    }
}
fn enchant(stack: &mut LootStack, name: &str, level: i32) {
    match stack.enchantments.iter_mut().find(|(n, _)| n == name) {
        Some(slot) => slot.1 = level,
        None => stack.enchantments.push((name.to_owned(), level)),
    }
}
