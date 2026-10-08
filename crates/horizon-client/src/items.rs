//! Items from vanilla's default-component reports, block<->item mapping,
//! tool mining rules and block loot tables (`LootTable`/`LootPool` semantics
//! for the condition, entry and modifier types used by block drops).
use crate::{Result, blocks::BlockFacts};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct ToolRule {
    pub blocks: Vec<String>,
    pub speed: Option<f32>,
    pub correct: Option<bool>,
}
#[derive(Clone, Debug)]
pub struct ItemInfo {
    pub id: Arc<str>,
    pub max_stack: u32,
    pub max_damage: u32,
    pub block: Option<BlockStateId>,
    pub tool: Vec<ToolRule>,
    pub tool_default_speed: f32,
    pub food: Option<(u32, f32)>,
    pub model: String,
    /// Main-hand `attack_damage` / `attack_speed` modifier amounts.
    pub attack_damage: f32,
    pub attack_speed: f32,
    /// `cooking_fuel`: burn-time and speed-multiplier provider references.
    pub fuel: Option<(Value, Value)>,
    /// `brewing_fuel`: uses and speed-multiplier provider references.
    pub brewing_fuel: Option<(Value, Value)>,
    /// Item left behind after crafting/fuel use (e.g. bucket).
    pub remainder: Option<String>,
    /// `equippable.slot` (head, chest, legs, feet, ...).
    pub equip_slot: Option<String>,
    /// All `attribute_modifiers`: (attribute, id, amount, operation, slot group).
    pub modifiers: Vec<(String, String, f64, String, String)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub item: Arc<str>,
    pub count: u32,
    pub damage: u32,
    /// Components beyond count and damage (see `item_data`); None when default.
    pub extra: Option<Box<crate::item_data::StackData>>,
}
pub struct Items {
    pub items: HashMap<Arc<str>, ItemInfo>,
    pub order: Vec<Arc<str>>,
    /// Block tag name (without '#') -> member block names.
    pub block_tags: HashMap<String, HashSet<String>>,
    pub item_tags: HashMap<String, HashSet<String>>,
    loot: HashMap<String, Value>,
}
fn load_tags(root: &Path, kind: &str) -> Result<HashMap<String, HashSet<String>>> {
    let dir = root.join(format!("data/minecraft/tags/{kind}"));
    let mut raw: HashMap<String, Vec<String>> = HashMap::new();
    fn walk(dir: &Path, base: &Path, raw: &mut HashMap<String, Vec<String>>) -> Result<()> {
        for entry in fs::read_dir(dir)?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, base, raw)?;
            } else if p.extension().is_some_and(|e| e == "json") {
                let name = format!(
                    "minecraft:{}",
                    p.strip_prefix(base)?.with_extension("").to_string_lossy().replace('\\', "/")
                );
                let v: Value = serde_json::from_slice(&fs::read(&p)?)?;
                let values = v["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str().or_else(|| e["id"].as_str()).map(str::to_owned))
                    .collect();
                raw.insert(name, values);
            }
        }
        Ok(())
    }
    if dir.exists() {
        walk(&dir, &dir, &mut raw)?;
    }
    fn resolve(name: &str, raw: &HashMap<String, Vec<String>>, out: &mut HashSet<String>, depth: u32) {
        if depth > 32 {
            return;
        }
        for v in raw.get(name).into_iter().flatten() {
            if let Some(tag) = v.strip_prefix('#') {
                resolve(tag, raw, out, depth + 1);
            } else {
                out.insert(v.clone());
            }
        }
    }
    Ok(raw
        .keys()
        .map(|k| {
            let mut set = HashSet::new();
            resolve(k, &raw, &mut set, 0);
            (k.clone(), set)
        })
        .collect())
}
impl Items {
    pub fn load(root: &Path, registry: &BlockRegistry, facts: &BlockFacts) -> Result<Self> {
        let raw: BTreeMap<String, Value> =
            serde_json::from_slice(&fs::read(root.join("items.json"))?)?;
        let mut block_for_item: HashMap<&str, BlockStateId> = HashMap::new();
        for (i, s) in facts.states.iter().enumerate() {
            let default = registry.default_state(&s.name);
            if default == Some(BlockStateId(i as u32)) && &*s.item != "minecraft:air" {
                block_for_item.entry(&s.item).or_insert(BlockStateId(i as u32));
            }
        }
        let mut items = HashMap::new();
        let mut order = Vec::new();
        for (name, c) in &raw {
            let id: Arc<str> = Arc::from(name.as_str());
            let tool = c["minecraft:tool"]["rules"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| ToolRule {
                    blocks: match &r["blocks"] {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
                        _ => vec![],
                    },
                    speed: r["speed"].as_f64().map(|v| v as f32),
                    correct: r["correct_for_drops"].as_bool(),
                })
                .collect();
            let modifier = |kind: &str| -> f32 {
                c["minecraft:attribute_modifiers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| m["type"].as_str() == Some(kind) && m["operation"].as_str() == Some("add_value"))
                    .filter_map(|m| m["amount"].as_f64())
                    .sum::<f64>() as f32
            };
            let food = c.get("minecraft:food").map(|f| {
                (
                    f["nutrition"].as_u64().unwrap_or(0) as u32,
                    f["saturation"].as_f64().unwrap_or(0.0) as f32,
                )
            });
            items.insert(
                id.clone(),
                ItemInfo {
                    id: id.clone(),
                    max_stack: c["minecraft:max_stack_size"].as_u64().unwrap_or(64) as u32,
                    max_damage: c["minecraft:max_damage"].as_u64().unwrap_or(0) as u32,
                    block: block_for_item.get(name.as_str()).copied(),
                    tool,
                    tool_default_speed: c["minecraft:tool"]["default_mining_speed"]
                        .as_f64()
                        .unwrap_or(1.0) as f32,
                    food,
                    model: c["minecraft:item_model"].as_str().unwrap_or(name).to_owned(),
                    equip_slot: c["minecraft:equippable"]["slot"].as_str().map(str::to_owned),
                    modifiers: c["minecraft:attribute_modifiers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| {
                            Some((
                                m["type"].as_str()?.to_owned(),
                                m["id"].as_str()?.to_owned(),
                                m["amount"].as_f64()?,
                                m["operation"].as_str().unwrap_or("add_value").to_owned(),
                                m["slot"].as_str().unwrap_or("any").to_owned(),
                            ))
                        })
                        .collect(),
                    fuel: c.get("minecraft:cooking_fuel").map(|f| (f["burn_time"].clone(), f["speed_multiplier"].clone())),
                    brewing_fuel: c.get("minecraft:brewing_fuel").map(|f| (f["uses"].clone(), f["speed_multiplier"].clone())),
                    remainder: c["minecraft:use_remainder"]["id"].as_str().or(c["minecraft:crafting_remainder"]["id"].as_str()).or(c["minecraft:crafting_remainder"].as_str()).map(str::to_owned),
                    attack_damage: modifier("minecraft:attack_damage"),
                    attack_speed: modifier("minecraft:attack_speed"),
                },
            );
            order.push(id);
        }
        let mut loot = HashMap::new();
        for kind in ["blocks", "entities"] {
            let dir = root.join(format!("data/minecraft/loot_table/{kind}"));
            for entry in fs::read_dir(&dir)?.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "json")
                    && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                {
                    loot.insert(format!("minecraft:{kind}/{stem}"), serde_json::from_slice(&fs::read(&p)?)?);
                }
            }
        }
        Ok(Self {
            items,
            order,
            block_tags: load_tags(root, "block")?,
            item_tags: load_tags(root, "item")?,
            loot,
        })
    }
    pub fn get(&self, id: &str) -> Option<&ItemInfo> {
        self.items.get(id)
    }
    /// An item table with only tags, for unit tests of item rules.
    #[cfg(test)]
    pub fn for_tests(item_tags: HashMap<String, HashSet<String>>) -> Self {
        Self { items: HashMap::new(), order: Vec::new(), block_tags: HashMap::new(), item_tags, loot: HashMap::new() }
    }
    fn block_matches(&self, spec: &str, block: &str) -> bool {
        match spec.strip_prefix('#') {
            Some(tag) => self.block_tags.get(tag).is_some_and(|s| s.contains(block)),
            None => spec == block,
        }
    }
    /// `Tool.getMiningSpeed` for the held item (1.0 without a tool component).
    pub fn mining_speed(&self, held: Option<&ItemStack>, block: &str) -> f32 {
        let Some(info) = held.and_then(|s| self.get(&s.item)) else {
            return 1.0;
        };
        for rule in &info.tool {
            if let Some(speed) = rule.speed
                && rule.blocks.iter().any(|b| self.block_matches(b, block))
            {
                return speed;
            }
        }
        if info.tool.is_empty() {
            1.0
        } else {
            info.tool_default_speed
        }
    }
    /// `Tool.isCorrectForDrops`.
    pub fn correct_for_drops(&self, held: Option<&ItemStack>, block: &str) -> bool {
        let Some(info) = held.and_then(|s| self.get(&s.item)) else {
            return false;
        };
        for rule in &info.tool {
            if let Some(correct) = rule.correct
                && rule.blocks.iter().any(|b| self.block_matches(b, block))
            {
                return correct;
            }
        }
        false
    }
    /// Evaluate the block's loot table for a player break without explosions.
    pub fn block_drops(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        held: Option<&ItemStack>,
        random: &mut LegacyRandom,
    ) -> Vec<ItemStack> {
        let Some(block) = registry.state(state) else {
            return vec![];
        };
        let name = block.name.trim_start_matches("minecraft:");
        let Some(table) = self.loot.get(&format!("minecraft:blocks/{name}")) else {
            return vec![];
        };
        let ctx = LootContext {
            items: self,
            block: &block.name,
            properties: &block.properties,
            held,
            killed_by_player: false,
        };
        ctx.evaluate(table, random)
    }
    /// Evaluate an entity's loot table (`minecraft:entities/<name>`).
    pub fn entity_drops(&self, entity: &str, killed_by_player: bool, held: Option<&ItemStack>, random: &mut LegacyRandom) -> Vec<ItemStack> {
        let name = entity.trim_start_matches("minecraft:");
        let Some(table) = self.loot.get(&format!("minecraft:entities/{name}")) else {
            return vec![];
        };
        let empty = BTreeMap::new();
        let ctx = LootContext {
            items: self,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
        };
        ctx.evaluate(table, random)
    }
}
impl LootContext<'_> {
    fn evaluate(&self, table: &Value, random: &mut LegacyRandom) -> Vec<ItemStack> {
        let ctx = self;
        let mut out = Vec::new();
        for pool in table["pools"].as_array().into_iter().flatten() {
            if !ctx.conditions(pool, random) {
                continue;
            }
            let rolls = ctx.number(&pool["rolls"], random).max(0.0) as u32;
            for _ in 0..rolls {
                let mut produced = Vec::new();
                ctx.choose(pool["entries"].as_array().map_or(&[][..], Vec::as_slice), random, &mut produced);
                for mut stack in produced {
                    ctx.modifiers(&pool["modifier"], &mut stack, random);
                    ctx.modifiers(&table["modifier"], &mut stack, random);
                    if stack.count > 0 {
                        out.push(stack);
                    }
                }
            }
        }
        out
    }
}
struct LootContext<'a> {
    items: &'a Items,
    block: &'a str,
    properties: &'a BTreeMap<String, String>,
    held: Option<&'a ItemStack>,
    killed_by_player: bool,
}
impl LootContext<'_> {
    /// Level of an enchantment on the tool / killer's weapon.
    fn enchantment_level(&self, id: &str) -> u32 {
        self.held.map_or(0, |h| h.enchantment_level(id))
    }
    fn number(&self, v: &Value, random: &mut LegacyRandom) -> f32 {
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
            Value::Object(o) => match o.get("type").and_then(Value::as_str).unwrap_or("minecraft:uniform") {
                "minecraft:constant" => o.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                "minecraft:uniform" => {
                    let min = self.number(o.get("min").unwrap_or(&Value::Null), random);
                    let max = self.number(o.get("max").unwrap_or(&Value::Null), random);
                    // UniformGenerator.getFloat / getInt (Mth.nextInt) differ; use int for counts.
                    if max <= min {
                        min
                    } else {
                        min + random.next_float() * (max - min)
                    }
                }
                "minecraft:binomial" => {
                    let n = self.number(o.get("n").unwrap_or(&Value::Null), random) as u32;
                    let p = self.number(o.get("p").unwrap_or(&Value::Null), random);
                    (0..n).filter(|_| random.next_float() < p).count() as f32
                }
                _ => 1.0,
            },
            _ => 1.0,
        }
    }
    fn int_number(&self, v: &Value, random: &mut LegacyRandom) -> i32 {
        if let Value::Object(o) = v
            && o.get("type").and_then(Value::as_str).unwrap_or("minecraft:uniform") == "minecraft:uniform"
        {
            let min = self.int_number(o.get("min").unwrap_or(&Value::Null), random);
            let max = self.int_number(o.get("max").unwrap_or(&Value::Null), random);
            return if max <= min {
                min
            } else {
                min + random.next_int_bound(max - min + 1).unwrap_or(0)
            };
        }
        self.number(v, random).round() as i32
    }
    fn condition(&self, c: &Value, random: &mut LegacyRandom) -> bool {
        let (kind, c) = match c {
            Value::String(s) => (s.as_str(), &Value::Null),
            Value::Object(o) => (o.get("type").and_then(Value::as_str).unwrap_or(""), c),
            _ => return true,
        };
        match kind {
            "minecraft:survives_explosion" => true,
            "minecraft:tool/can_silk_touch" => self.enchantment_level("minecraft:silk_touch") > 0,
            "minecraft:tool/can_shear" => self.held.is_some_and(|h| &*h.item == "minecraft:shears"),
            "minecraft:inverted" => !self.condition(&c["term"], random),
            "minecraft:any_of" => c["terms"].as_array().into_iter().flatten().any(|t| self.condition(t, random)),
            "minecraft:all_of" => c["terms"].as_array().into_iter().flatten().all(|t| self.condition(t, random)),
            "minecraft:random_chance" => random.next_float() < self.number(&c["chance"], random),
            "minecraft:table_bonus" => {
                // BonusLevelTableCondition: chances[min(level, len - 1)].
                let level = self.enchantment_level(c["enchantment"].as_str().unwrap_or("")) as usize;
                let chances = c["chances"].as_array().map_or(&[][..], Vec::as_slice);
                let chance = chances.get(level.min(chances.len().saturating_sub(1))).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                random.next_float() < chance
            }
            "minecraft:match_block" | "minecraft:block_state_property" => {
                let blocks_ok = match &c["blocks"].as_str().or(c["block"].as_str()) {
                    Some(spec) => self.items.block_matches(spec, self.block),
                    None => true,
                };
                blocks_ok
                    && c["state"].as_object().into_iter().flatten().chain(c["properties"].as_object().into_iter().flatten()).all(|(k, v)| {
                        let have = self.properties.get(k).map(String::as_str);
                        match v {
                            Value::String(s) => have == Some(s.as_str()),
                            Value::Object(r) => {
                                let have: i64 = have.and_then(|h| h.parse().ok()).unwrap_or(0);
                                r.get("min").and_then(Value::as_i64).is_none_or(|m| have >= m)
                                    && r.get("max").and_then(Value::as_i64).is_none_or(|m| have <= m)
                            }
                            _ => true,
                        }
                    })
            }
            "minecraft:match_tool" => {
                let items = &c["predicate"]["items"];
                let items_ok = match (self.held, items) {
                    (_, Value::Null) => self.held.is_some(),
                    (Some(h), Value::String(s)) => match s.strip_prefix('#') {
                        Some(tag) => self.items.item_tags.get(tag).is_some_and(|t| t.contains(&*h.item)),
                        None => **s == *h.item,
                    },
                    _ => false,
                };
                // ItemPredicate "minecraft:enchantments": each entry needs a level in range.
                let enchants_ok = c["predicate"]["predicates"]["minecraft:enchantments"].as_array().into_iter().flatten().all(|e| {
                    let level = self.enchantment_level(e["enchantments"].as_str().unwrap_or(""));
                    let min = e["levels"]["min"].as_u64().unwrap_or(1) as u32;
                    let max = e["levels"]["max"].as_u64().map_or(u32::MAX, |m| m as u32);
                    level >= min.max(1) && level <= max
                });
                items_ok && enchants_ok
            }
            "minecraft:killed_by_player" => self.killed_by_player,
            "minecraft:random_chance_with_enchanted_bonus" => {
                let level = if self.killed_by_player && self.block.is_empty() { self.enchantment_level(c["enchantment"].as_str().unwrap_or("")) } else { 0 };
                let chance = if level > 0 {
                    crate::enchant::level_value(&c["enchanted_chance"], level)
                } else {
                    c["unenchanted_chance"].as_f64().unwrap_or(0.0) as f32
                };
                random.next_float() < chance
            }
            // Entity flags, equipment and damage-source predicates: none hold
            // for an unenchanted player kill without fire or lightning.
            "minecraft:entity_properties" | "minecraft:damage_source_properties" => false,
            _ => true,
        }
    }
    fn conditions(&self, o: &Value, random: &mut LegacyRandom) -> bool {
        if let Some(c) = o.get("condition")
            && !self.condition(c, random)
        {
            return false;
        }
        o.get("conditions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .all(|c| self.condition(c, random))
    }
    /// Expand entries; returns whether an entry succeeded (for alternatives).
    fn expand(&self, e: &Value, random: &mut LegacyRandom, out: &mut Vec<(i32, ItemStack, Value)>) -> bool {
        if !self.conditions(e, random) {
            return false;
        }
        match e["type"].as_str().unwrap_or("") {
            "minecraft:item" => {
                out.push((
                    e["weight"].as_i64().unwrap_or(1) as i32,
                    ItemStack {
                        item: Arc::from(e["name"].as_str().unwrap_or("minecraft:air")),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    e["modifier"].clone(),
                ));
                true
            }
            "minecraft:alternatives" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    if self.expand(child, random, out) {
                        return true;
                    }
                }
                false
            }
            "minecraft:group" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    self.expand(child, random, out);
                }
                true
            }
            "minecraft:sequence" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    if !self.expand(child, random, out) {
                        break;
                    }
                }
                true
            }
            "minecraft:empty" => {
                out.push((e["weight"].as_i64().unwrap_or(1) as i32, ItemStack { item: Arc::from("minecraft:air"), count: 0, damage: 0, extra: None }, Value::Null));
                true
            }
            _ => false,
        }
    }
    fn choose(&self, entries: &[Value], random: &mut LegacyRandom, produced: &mut Vec<ItemStack>) {
        let mut candidates = Vec::new();
        for e in entries {
            self.expand(e, random, &mut candidates);
        }
        if candidates.is_empty() {
            return;
        }
        let pick = if candidates.len() == 1 {
            0
        } else {
            let total: i32 = candidates.iter().map(|c| c.0.max(0)).sum();
            let mut r = random.next_int_bound(total.max(1)).unwrap_or(0);
            let mut chosen = 0;
            for (i, c) in candidates.iter().enumerate() {
                r -= c.0.max(0);
                if r < 0 {
                    chosen = i;
                    break;
                }
            }
            chosen
        };
        let (_, mut stack, modifiers) = candidates.swap_remove(pick);
        self.modifiers(&modifiers, &mut stack, random);
        produced.push(stack);
    }
    fn modifiers(&self, m: &Value, stack: &mut ItemStack, random: &mut LegacyRandom) {
        let list: Vec<&Value> = match m {
            Value::Array(a) => a.iter().collect(),
            Value::Object(_) => vec![m],
            _ => return,
        };
        for f in list {
            if !self.conditions(f, random) {
                continue;
            }
            match f["type"].as_str().or(f["function"].as_str()).unwrap_or("") {
                "minecraft:set_count" => {
                    let n = self.int_number(&f["count"], random);
                    let base = if f["add"].as_bool() == Some(true) { stack.count as i32 } else { 0 };
                    stack.count = (base + n).max(0) as u32;
                }
                "minecraft:apply_bonus" => {
                    let level = self.enchantment_level(f["enchantment"].as_str().unwrap_or("")) as i32;
                    let count = stack.count as i32;
                    let params = &f["parameters"];
                    stack.count = match f["formula"].as_str().unwrap_or("") {
                        "minecraft:ore_drops" if level > 0 => {
                            let bonus = (random.next_int_bound(level + 2).unwrap_or(0) - 1).max(0);
                            count * (bonus + 1)
                        }
                        "minecraft:uniform_bonus_count" => {
                            let multiplier = params["bonusMultiplier"].as_i64().unwrap_or(1) as i32;
                            count + random.next_int_bound(multiplier * level + 1).unwrap_or(0)
                        }
                        "minecraft:binomial_with_bonus_count" => {
                            let (extra, p) = (params["extra"].as_i64().unwrap_or(0) as i32, params["probability"].as_f64().unwrap_or(0.0) as f32);
                            let mut n = count;
                            for _ in 0..level + extra {
                                if random.next_float() < p {
                                    n += 1;
                                }
                            }
                            n
                        }
                        _ => count,
                    }
                    .max(0) as u32;
                }
                "minecraft:enchanted_count_increase" => {
                    // Attacker's level x count, rounded (looting).
                    let level = self.enchantment_level(f["enchantment"].as_str().unwrap_or(""));
                    if level > 0 && self.killed_by_player && self.block.is_empty() {
                        let add = level as f32 * self.number(&f["count"], random);
                        stack.count = (stack.count as i32 + add.round() as i32).max(0) as u32;
                        if let Some(limit) = f["limit"].as_i64().filter(|l| *l > 0) {
                            stack.count = stack.count.min(limit as u32);
                        }
                    }
                }
                "minecraft:limit_count" => {
                    let limit = &f["limit"];
                    if let Some(min) = limit["min"].as_i64() {
                        stack.count = stack.count.max(min as u32);
                    }
                    if let Some(max) = limit["max"].as_i64() {
                        stack.count = stack.count.min(max as u32);
                    }
                }
                _ => {}
            }
        }
    }
}
