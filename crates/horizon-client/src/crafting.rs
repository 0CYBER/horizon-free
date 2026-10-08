//! Crafting and cooking recipes from the pinned data pack: shaped patterns
//! (placed anywhere in the grid, optionally mirrored as in
//! `ShapedRecipePattern.matches`), shapeless ingredient sets, and furnace-type
//! cooking lookups.
use crate::{Result, items::{ItemStack, Items}};
use serde_json::Value;
use std::{collections::HashSet, fs, path::Path, sync::Arc};

/// A set of accepted items (expanded from an item, list or tag).
#[derive(Clone, Debug)]
pub struct Ingredient(pub HashSet<String>);
impl Ingredient {
    fn parse(v: &Value, items: &Items) -> Option<Self> {
        let mut set = HashSet::new();
        let mut add = |s: &str| {
            if let Some(tag) = s.strip_prefix('#') {
                set.extend(items.item_tags.get(tag).into_iter().flatten().cloned());
            } else {
                set.insert(s.to_owned());
            }
        };
        match v {
            Value::String(s) => add(s),
            Value::Array(a) => a.iter().filter_map(Value::as_str).for_each(&mut add),
            Value::Object(o) => {
                if let Some(s) = o.get("item").and_then(Value::as_str) {
                    add(s);
                }
                if let Some(t) = o.get("tag").and_then(Value::as_str) {
                    add(&format!("#{t}"));
                }
            }
            _ => return None,
        }
        (!set.is_empty()).then_some(Self(set))
    }
    pub fn test(&self, stack: &ItemStack) -> bool {
        self.0.contains(&*stack.item)
    }
}
#[derive(Clone, Debug)]
pub enum Recipe {
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<Ingredient>>,
        result: ItemStack,
    },
    Shapeless {
        ingredients: Vec<Ingredient>,
        result: ItemStack,
    },
}
#[derive(Clone, Debug)]
pub struct Cooking {
    pub kind: String,
    pub ingredient: Ingredient,
    pub result: ItemStack,
    pub experience: f32,
    pub time: u32,
}
pub struct Recipes {
    pub crafting: Vec<(String, Recipe)>,
    pub cooking: Vec<Cooking>,
}
fn result(v: &Value) -> Option<ItemStack> {
    Some(ItemStack {
        item: Arc::from(v["id"].as_str()?),
        count: v["count"].as_u64().unwrap_or(1) as u32,
        damage: 0,
        extra: None,
    })
}
impl Recipes {
    pub fn load(root: &Path, items: &Items) -> Result<Self> {
        let mut crafting = Vec::new();
        let mut cooking = Vec::new();
        let mut files: Vec<_> = fs::read_dir(root.join("data/minecraft/recipe"))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        for path in files {
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
            let v: Value = serde_json::from_slice(&fs::read(&path)?)?;
            match v["type"].as_str().unwrap_or("") {
                "minecraft:crafting_shaped" => {
                    let pattern: Vec<Vec<char>> = v["pattern"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|r| r.as_str().map(|s| s.chars().collect()))
                        .collect();
                    let height = pattern.len();
                    let width = pattern.iter().map(Vec::len).max().unwrap_or(0);
                    let mut cells = Vec::with_capacity(width * height);
                    let mut ok = true;
                    for row in &pattern {
                        for x in 0..width {
                            let c = row.get(x).copied().unwrap_or(' ');
                            if c == ' ' {
                                cells.push(None);
                            } else {
                                match Ingredient::parse(&v["key"][c.to_string()], items) {
                                    Some(i) => cells.push(Some(i)),
                                    None => ok = false,
                                }
                            }
                        }
                    }
                    if let (true, Some(result)) = (ok, result(&v["result"])) {
                        crafting.push((name, Recipe::Shaped { width, height, cells, result }));
                    }
                }
                "minecraft:crafting_shapeless" => {
                    let ingredients: Option<Vec<_>> = v["ingredients"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|i| Ingredient::parse(i, items))
                        .collect();
                    if let (Some(ingredients), Some(result)) = (ingredients, result(&v["result"])) {
                        crafting.push((name, Recipe::Shapeless { ingredients, result }));
                    }
                }
                kind @ ("minecraft:smelting" | "minecraft:blasting" | "minecraft:smoking" | "minecraft:campfire_cooking") => {
                    if let (Some(ingredient), Some(result)) = (Ingredient::parse(&v["ingredient"], items), result(&v["result"])) {
                        cooking.push(Cooking {
                            kind: kind.to_owned(),
                            ingredient,
                            result,
                            experience: v["experience"].as_f64().unwrap_or(0.0) as f32,
                            time: v["cookingtime"].as_u64().unwrap_or(200) as u32,
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(Self { crafting, cooking })
    }
    /// Match a `size`x`size` grid (row-major) against the crafting recipes.
    pub fn craft(&self, grid: &[Option<ItemStack>], size: usize) -> Option<ItemStack> {
        // Trim to the occupied bounding box.
        let occupied: Vec<(usize, usize)> = (0..size * size)
            .filter(|&i| grid[i].is_some())
            .map(|i| (i % size, i / size))
            .collect();
        if occupied.is_empty() {
            return None;
        }
        let (x0, x1) = (occupied.iter().map(|p| p.0).min()?, occupied.iter().map(|p| p.0).max()?);
        let (y0, y1) = (occupied.iter().map(|p| p.1).min()?, occupied.iter().map(|p| p.1).max()?);
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let cell = |x: usize, y: usize| grid[(y0 + y) * size + x0 + x].as_ref();
        for (_, recipe) in &self.crafting {
            match recipe {
                Recipe::Shaped { width, height, cells, result } => {
                    if *width != w || *height != h {
                        continue;
                    }
                    let matches = |mirror: bool| {
                        (0..h).all(|y| {
                            (0..w).all(|x| {
                                let rx = if mirror { w - 1 - x } else { x };
                                match (&cells[y * w + rx], cell(x, y)) {
                                    (None, None) => true,
                                    (Some(i), Some(s)) => i.test(s),
                                    _ => false,
                                }
                            })
                        })
                    };
                    if matches(false) || matches(true) {
                        return Some(result.clone());
                    }
                }
                Recipe::Shapeless { ingredients, result } => {
                    let stacks: Vec<&ItemStack> = grid.iter().flatten().collect();
                    if stacks.len() != ingredients.len() {
                        continue;
                    }
                    // Greedy bipartite matching is sufficient for vanilla's ingredient sets.
                    let mut used = vec![false; stacks.len()];
                    let ok = ingredients.iter().all(|ingredient| {
                        if let Some(i) = (0..stacks.len()).find(|&i| !used[i] && ingredient.test(stacks[i])) {
                            used[i] = true;
                            true
                        } else {
                            false
                        }
                    });
                    if ok {
                        return Some(result.clone());
                    }
                }
            }
        }
        None
    }
    pub fn cook(&self, kind: &str, input: &ItemStack) -> Option<&Cooking> {
        self.cooking.iter().find(|c| c.kind == kind && c.ingredient.test(input))
    }
}
