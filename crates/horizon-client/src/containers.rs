//! Container block entities: chests/barrels (27 slots) and the three furnace
//! kinds, ticking as `AbstractFurnaceBlockEntity.serverTick` with fuel times
//! from the data-driven `context_int_provider`/`context_float_provider` files.
use crate::{
    crafting::Recipes,
    items::{ItemStack, Items},
};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FurnaceKind {
    Furnace,
    BlastFurnace,
    Smoker,
}
impl FurnaceKind {
    pub fn recipe_type(self) -> &'static str {
        match self {
            Self::Furnace => "minecraft:smelting",
            Self::BlastFurnace => "minecraft:blasting",
            Self::Smoker => "minecraft:smoking",
        }
    }
    pub fn fast(self) -> bool {
        self != Self::Furnace
    }
    pub fn from_block(name: &str) -> Option<Self> {
        match name {
            "minecraft:furnace" => Some(Self::Furnace),
            "minecraft:blast_furnace" => Some(Self::BlastFurnace),
            "minecraft:smoker" => Some(Self::Smoker),
            _ => None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Furnace {
    pub kind: FurnaceKind,
    /// input, fuel, result
    pub items: [Option<ItemStack>; 3],
    pub lit_remaining: i32,
    pub lit_total: i32,
    pub cooking_timer: i32,
    pub cooking_total: i32,
    pub speed: f32,
}
#[derive(Clone, Debug)]
pub enum BlockEntity {
    Chest(Vec<Option<ItemStack>>),
    Furnace(Furnace),
    Brewing(Box<crate::brewing::BrewingStand>),
    /// Signs and hanging signs (`SignBlockEntity`).
    Sign(crate::signs::SignData),
}
impl BlockEntity {
    pub fn for_block(name: &str) -> Option<Self> {
        if let Some(hanging) = crate::signs::sign_kind(name) {
            return Some(Self::Sign(crate::signs::SignData { hanging, ..Default::default() }));
        }
        if let Some(kind) = FurnaceKind::from_block(name) {
            return Some(Self::Furnace(Furnace {
                kind,
                items: [None, None, None],
                lit_remaining: 0,
                lit_total: 0,
                cooking_timer: 0,
                cooking_total: 200,
                speed: 1.0,
            }));
        }
        if name == "minecraft:brewing_stand" {
            return Some(Self::Brewing(Box::new(crate::brewing::BrewingStand::new())));
        }
        matches!(
            name,
            "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel"
        )
        .then(|| Self::Chest(vec![None; 27]))
    }
    pub fn id(&self) -> &'static str {
        match self {
            Self::Chest(_) => "minecraft:chest",
            Self::Brewing(_) => "minecraft:brewing_stand",
        let size = match name {
            "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" => 27,
            "minecraft:hopper" => 5,
            "minecraft:dispenser" | "minecraft:dropper" => 9,
            _ => return None,
        };
        Some(Self::Chest(vec![None; size]))
    }
    /// The vanilla block entity type id of a container block.
    pub fn id_for_block(name: &str) -> Option<&'static str> {
        Some(match name {
            "minecraft:chest" => "minecraft:chest",
            "minecraft:trapped_chest" => "minecraft:trapped_chest",
            "minecraft:barrel" => "minecraft:barrel",
            "minecraft:hopper" => "minecraft:hopper",
            "minecraft:dispenser" => "minecraft:dispenser",
            "minecraft:dropper" => "minecraft:dropper",
            "minecraft:furnace" => "minecraft:furnace",
            "minecraft:blast_furnace" => "minecraft:blast_furnace",
            "minecraft:smoker" => "minecraft:smoker",
            _ => return None,
        })
    }
    pub fn id(&self) -> &'static str {
        match self {
            Self::Chest(items) => match items.len() {
                5 => "minecraft:hopper",
                9 => "minecraft:dispenser",
                _ => "minecraft:chest",
            },
            Self::Furnace(f) => match f.kind {
                FurnaceKind::Furnace => "minecraft:furnace",
                FurnaceKind::BlastFurnace => "minecraft:blast_furnace",
                FurnaceKind::Smoker => "minecraft:smoker",
            },
            Self::Sign(s) => s.id(),
        }
    }
    pub fn contents(&self) -> Vec<ItemStack> {
        match self {
            Self::Chest(items) => items.iter().flatten().cloned().collect(),
            Self::Furnace(f) => f.items.iter().flatten().cloned().collect(),
            Self::Brewing(b) => b.items.iter().flatten().cloned().collect(),
        }
    }
    pub fn slots(&self) -> &[Option<ItemStack>] {
        match self {
            Self::Chest(items) => items,
            Self::Furnace(f) => &f.items,
            Self::Sign(_) => Vec::new(),
        }
    }
    pub fn slots_mut(&mut self) -> &mut [Option<ItemStack>] {
        match self {
            Self::Chest(items) => items,
            Self::Furnace(f) => &mut f.items,
            Self::Brewing(b) => &mut b.items,
            Self::Sign(_) => &mut [],
        }
    }
}
/// Evaluator for `context_int_provider` / `context_float_provider` values.
pub struct Providers {
    ints: HashMap<String, Value>,
    floats: HashMap<String, Value>,
}
fn load_dir(dir: &Path, prefix: &str, out: &mut HashMap<String, Value>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
        if p.is_dir() {
            load_dir(&p, &format!("{prefix}{name}/"), out);
        } else if let Ok(bytes) = fs::read(&p)
            && let Ok(v) = serde_json::from_slice(&bytes)
        {
            out.insert(format!("minecraft:{prefix}{name}"), v);
        }
    }
}
impl Providers {
    pub fn load(root: &Path) -> Self {
        let mut ints = HashMap::new();
        let mut floats = HashMap::new();
        load_dir(&root.join("data/minecraft/context_int_provider"), "", &mut ints);
        load_dir(&root.join("data/minecraft/context_float_provider"), "", &mut floats);
        Self { ints, floats }
    }
    fn eval(&self, v: &Value, fast: bool, table: &HashMap<String, Value>, depth: u32) -> f64 {
        if depth > 16 {
            return 0.0;
        }
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            Value::String(name) => table.get(name).map_or(0.0, |v| self.eval(v, fast, table, depth + 1)),
            Value::Object(o) => {
                let f = |k: &str| self.eval(o.get(k).unwrap_or(&Value::Null), fast, table, depth + 1);
                match o.get("type").and_then(Value::as_str).unwrap_or("") {
                    "minecraft:div" => {
                        let r = f("right");
                        if r == 0.0 { 0.0 } else { (f("left") / r).trunc() }
                    }
                    "minecraft:mul" => f("left") * f("right"),
                    "minecraft:add" | "minecraft:sum" => f("left") + f("right"),
                    "minecraft:sub" => f("left") - f("right"),
                    "minecraft:constant" => f("value"),
                    "minecraft:conditional" => {
                        // The only condition in the pinned data: block/fast_cooking.
                        let condition = o.get("condition").and_then(Value::as_str) == Some("minecraft:block/fast_cooking");
                        if condition && fast { f("on_true") } else { f("on_false") }
                    }
                    _ => 0.0,
                }
            }
            _ => 0.0,
        }
    }
    /// A plain `context_int_provider` value or reference (no context conditions).
    pub fn int_value(&self, v: &Value) -> i32 {
        self.eval(v, false, &self.ints, 0) as i32
    }
    /// A plain `context_float_provider` value or reference; `default` when absent.
    pub fn float_value(&self, v: &Value, default: f32) -> f32 {
        if v.is_null() { default } else { self.eval(v, false, &self.floats, 0) as f32 }
    }
    #[cfg(test)]
    pub fn empty() -> Self {
        Self { ints: HashMap::new(), floats: HashMap::new() }
    }
    pub fn burn_time(&self, items: &Items, stack: &ItemStack, fast: bool) -> i32 {
        items
            .get(&stack.item)
            .and_then(|i| i.fuel.as_ref())
            .map_or(0, |(time, _)| self.eval(time, fast, &self.ints, 0) as i32)
    }
    pub fn speed(&self, items: &Items, stack: &ItemStack, fast: bool) -> f32 {
        items
            .get(&stack.item)
            .and_then(|i| i.fuel.as_ref())
            .map_or(1.0, |(_, speed)| self.eval(speed, fast, &self.floats, 0) as f32)
    }
}
fn remainder(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:lava_bucket" | "minecraft:water_bucket" | "minecraft:milk_bucket" => Some("minecraft:bucket"),
        _ => None,
    }
}
impl Furnace {
    /// One server tick. Returns `Some(lit)` when the LIT block state must change.
    pub fn tick(&mut self, recipes: &Recipes, items: &Items, providers: &Providers) -> Option<bool> {
        let was_lit = self.lit_remaining > 0;
        let mut lit = false;
        if self.lit_remaining > 0 {
            self.lit_remaining -= 1;
            lit = self.lit_remaining > 0;
        }
        let fast = self.kind.fast();
        let has_ingredient = self.items[0].is_some();
        let has_fuel = self.items[1].is_some();
        if lit || has_fuel && has_ingredient {
            if let Some(ingredient) = self.items[0].clone() {
                if let Some(recipe) = recipes.cook(self.kind.recipe_type(), &ingredient).cloned() {
                    if self.cooking_timer == 0 {
                        self.cooking_total = total_time(recipe.time, self.speed);
                    }
                    let max = items.get(&recipe.result.item).map_or(64, |i| i.max_stack);
                    let can_burn = match &self.items[2] {
                        None => true,
                        Some(out) => out.item == recipe.result.item && out.count + recipe.result.count <= max,
                    };
                    if can_burn {
                        if !lit {
                            let fuel = self.items[1].clone();
                            let time = fuel.as_ref().map_or(0, |f| providers.burn_time(items, f, fast));
                            let speed = fuel.as_ref().map_or(1.0, |f| providers.speed(items, f, fast));
                            self.lit_remaining = time;
                            self.lit_total = time;
                            self.speed = speed;
                            if self.cooking_total > 0 && self.cooking_timer < self.cooking_total {
                                let ratio = self.cooking_timer as f32 / self.cooking_total as f32;
                                self.cooking_total = total_time(recipe.time, self.speed);
                                self.cooking_timer = (ratio * self.cooking_total as f32).ceil() as i32;
                            }
                            if time > 0 {
                                if let Some(f) = self.items[1].as_mut() {
                                    let item = f.item.clone();
                                    f.count -= 1;
                                    if f.count == 0 {
                                        self.items[1] = remainder(&item).map(|r| ItemStack { item: Arc::from(r), count: 1, damage: 0, extra: None });
                                    }
                                }
                                lit = true;
                            }
                        }
                        if lit {
                            self.cooking_timer += 1;
                            if self.cooking_timer >= self.cooking_total {
                                self.cooking_timer = 0;
                                self.cooking_total = total_time(recipe.time, self.speed);
                                match &mut self.items[2] {
                                    Some(out) => out.count += recipe.result.count,
                                    None => self.items[2] = Some(recipe.result.clone()),
                                }
                                if let Some(i) = self.items[0].as_mut() {
                                    i.count -= 1;
                                    if i.count == 0 {
                                        self.items[0] = None;
                                    }
                                }
                            }
                        } else {
                            self.cooking_timer = 0;
                        }
                    } else {
                        self.cooking_timer = 0;
                    }
                }
            } else {
                self.cooking_timer = 0;
            }
        } else if self.cooking_timer > 0 {
            self.cooking_timer = (self.cooking_timer - 2).clamp(0, self.cooking_total);
        }
        (was_lit != lit).then_some(lit)
    }
}
fn total_time(time: u32, speed: f32) -> i32 {
    if speed > 0.0 {
        (time as f32 / speed).ceil() as i32
    } else {
        time as i32
    }
}
