//! Crafting and cooking recipes from the pinned data pack: shaped patterns
//! (placed anywhere in the grid, optionally mirrored as in
//! `ShapedRecipePattern.matches`), shapeless ingredient sets, and furnace-type
//! cooking lookups.
use crate::{
    Result,
    items::{ItemStack, Items},
};
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
    /// `BannerDuplicateRecipe` and `ShieldDecorationRecipe`.
    Special(Special),
}
/// The banner-copying custom recipes.
#[derive(Clone, Debug)]
pub enum Special {
    /// A patterned banner and a blank one of the same colour copy the patterns.
    BannerDuplicate { banner: Ingredient, result: String },
    /// A banner decorates an undecorated shield.
    ShieldDecoration {
        banner: Ingredient,
        target: Ingredient,
        result: String,
    },
}
impl Special {
    /// `matches` + `assemble` over the non-empty stacks of the grid.
    fn assemble(&self, stacks: &[&ItemStack]) -> Option<ItemStack> {
        use crate::banner;
        if stacks.len() != 2 {
            return None;
        }
        match self {
            Self::BannerDuplicate {
                banner: ing,
                result,
            } => {
                let color = banner::banner_color(&stacks[0].item)?;
                let mut source = None;
                let mut target = false;
                for s in stacks {
                    if !ing.test(s) || banner::banner_color(&s.item)? != color {
                        return None;
                    }
                    let layers = banner::layers(s).len();
                    if layers > banner::MAX_LAYERS {
                        return None;
                    }
                    if layers > 0 {
                        if source.is_some() {
                            return None;
                        }
                        source = Some(*s);
                    } else {
                        if target {
                            return None;
                        }
                        target = true;
                    }
                }
                let source = source.filter(|_| target)?;
                // TransmuteRecipe.createWithOriginalComponents: the source's components.
                Some(ItemStack {
                    item: Arc::from(result.as_str()),
                    count: 1,
                    ..(*source).clone()
                })
            }
            Self::ShieldDecoration {
                banner: ing,
                target: tgt,
                result,
            } => {
                let (mut patterned, mut clear) = (None, None);
                for s in stacks {
                    if ing.test(s) && banner::banner_color(&s.item).is_some() {
                        if patterned.is_some() {
                            return None;
                        }
                        patterned = Some(*s);
                    } else {
                        if !tgt.test(s) || clear.is_some() || !banner::layers(s).is_empty() {
                            return None;
                        }
                        clear = Some(*s);
                    }
                }
                let (banner_stack, shield) = (patterned?, clear?);
                let mut out = ItemStack {
                    item: Arc::from(result.as_str()),
                    count: 1,
                    ..shield.clone()
                };
                let data = out.data_mut();
                data.banner_patterns = banner::layers(banner_stack).to_vec();
                data.base_color = banner::banner_color(&banner_stack.item).map(Arc::from);
                Some(out)
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Cooking {
    /// The recipe's file name (its id without namespace).
    pub id: String,
    /// `category` (`food`, `blocks`, `misc`) and `group` of the recipe book.
    pub category: String,
    pub group: String,
    pub kind: String,
    pub ingredient: Ingredient,
    pub result: ItemStack,
    pub experience: f32,
    pub time: u32,
}
pub struct Recipes {
    pub crafting: Vec<(String, Recipe)>,
    pub cooking: Vec<Cooking>,
    /// `category` and `group` of each crafting recipe, by file name.
    pub meta: std::collections::HashMap<String, (String, String)>,
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
        let mut meta = std::collections::HashMap::new();
        let mut files: Vec<_> = fs::read_dir(root.join("data/minecraft/recipe"))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned();
            let v: Value = serde_json::from_slice(&fs::read(&path)?)?;
            let text = |k: &str| v[k].as_str().unwrap_or("").to_owned();
            meta.insert(name.clone(), (text("category"), text("group")));
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
                        crafting.push((
                            name,
                            Recipe::Shaped {
                                width,
                                height,
                                cells,
                                result,
                            },
                        ));
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
                        crafting.push((
                            name,
                            Recipe::Shapeless {
                                ingredients,
                                result,
                            },
                        ));
                    }
                }
                "minecraft:crafting_special_bannerduplicate" => {
                    if let (Some(banner), Some(id)) = (
                        Ingredient::parse(&v["banner"], items),
                        v["result"]["id"].as_str(),
                    ) {
                        crafting.push((
                            name,
                            Recipe::Special(Special::BannerDuplicate {
                                banner,
                                result: id.to_owned(),
                            }),
                        ));
                    }
                }
                "minecraft:crafting_special_shielddecoration" => {
                    if let (Some(banner), Some(target), Some(id)) = (
                        Ingredient::parse(&v["banner"], items),
                        Ingredient::parse(&v["target"], items),
                        v["result"]["id"].as_str(),
                    ) {
                        crafting.push((
                            name,
                            Recipe::Special(Special::ShieldDecoration {
                                banner,
                                target,
                                result: id.to_owned(),
                            }),
                        ));
                    }
                }
                kind @ ("minecraft:smelting"
                | "minecraft:blasting"
                | "minecraft:smoking"
                | "minecraft:campfire_cooking") => {
                    if let (Some(ingredient), Some(result)) = (
                        Ingredient::parse(&v["ingredient"], items),
                        result(&v["result"]),
                    ) {
                        cooking.push(Cooking {
                            id: name,
                            category: text("category"),
                            group: text("group"),
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
        Ok(Self {
            crafting,
            cooking,
            meta,
        })
    }
    /// Match a `size`x`size` grid (row-major) against the crafting recipes.
    pub fn craft(&self, grid: &[Option<ItemStack>], size: usize) -> Option<ItemStack> {
        self.find(grid, size).map(|(_, result)| result)
    }
    /// `craft` with the matched recipe's id (the file name without namespace).
    pub fn find(&self, grid: &[Option<ItemStack>], size: usize) -> Option<(&str, ItemStack)> {
        // Trim to the occupied bounding box.
        let occupied: Vec<(usize, usize)> = (0..size * size)
            .filter(|&i| grid[i].is_some())
            .map(|i| (i % size, i / size))
            .collect();
        if occupied.is_empty() {
            return None;
        }
        let (x0, x1) = (
            occupied.iter().map(|p| p.0).min()?,
            occupied.iter().map(|p| p.0).max()?,
        );
        let (y0, y1) = (
            occupied.iter().map(|p| p.1).min()?,
            occupied.iter().map(|p| p.1).max()?,
        );
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let cell = |x: usize, y: usize| grid[(y0 + y) * size + x0 + x].as_ref();
        for (name, recipe) in &self.crafting {
            match recipe {
                Recipe::Special(special) => {
                    let stacks: Vec<&ItemStack> = grid.iter().flatten().collect();
                    if let Some(result) = special.assemble(&stacks) {
                        return Some((name, result));
                    }
                }
                Recipe::Shaped {
                    width,
                    height,
                    cells,
                    result,
                } => {
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
                        return Some((name, result.clone()));
                    }
                }
                Recipe::Shapeless {
                    ingredients,
                    result,
                } => {
                    let stacks: Vec<&ItemStack> = grid.iter().flatten().collect();
                    if stacks.len() != ingredients.len() {
                        continue;
                    }
                    // Greedy bipartite matching is sufficient for vanilla's ingredient sets.
                    let mut used = vec![false; stacks.len()];
                    let ok = ingredients.iter().all(|ingredient| {
                        if let Some(i) =
                            (0..stacks.len()).find(|&i| !used[i] && ingredient.test(stacks[i]))
                        {
                            used[i] = true;
                            true
                        } else {
                            false
                        }
                    });
                    if ok {
                        return Some((name, result.clone()));
                    }
                }
            }
        }
        None
    }
    /// `Recipe.getRemainingItems` per grid cell: an item's crafting remainder
    /// (buckets, bottles), and the patterned banner a duplicate keeps.
    pub fn remainders(
        &self,
        items: &Items,
        grid: &[Option<ItemStack>],
        result: &ItemStack,
    ) -> Vec<Option<ItemStack>> {
        grid.iter()
            .map(|cell| {
                let s = cell.as_ref()?;
                if let Some(rem) = items.get(&s.item).and_then(|i| i.remainder.as_deref()) {
                    return Some(ItemStack::new(rem, 1));
                }
                (result.item.ends_with("_banner") && !crate::banner::layers(s).is_empty()).then(
                    || ItemStack {
                        count: 1,
                        ..s.clone()
                    },
                )
            })
            .collect()
    }
    pub fn cook(&self, kind: &str, input: &ItemStack) -> Option<&Cooking> {
        self.cooking
            .iter()
            .find(|c| c.kind == kind && c.ingredient.test(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::banner::BannerLayer;

    fn ingredient(items: &[&str]) -> Ingredient {
        Ingredient(items.iter().map(|s| (*s).to_owned()).collect())
    }
    fn patterned(item: &str) -> ItemStack {
        let mut s = ItemStack::new(item, 1);
        s.data_mut().banner_patterns = vec![BannerLayer::new("minecraft:cross", "red")];
        s
    }
    #[test]
    fn banner_duplicate_copies_the_patterns_of_the_same_colour() {
        let recipe = Special::BannerDuplicate {
            banner: ingredient(&["minecraft:white_banner"]),
            result: "minecraft:white_banner".into(),
        };
        let source = patterned("minecraft:white_banner");
        let blank = ItemStack::new("minecraft:white_banner", 1);
        let out = recipe.assemble(&[&source, &blank]).expect("copy");
        assert_eq!(out.data().unwrap().banner_patterns.len(), 1);
        // Two blanks, or two patterned banners, do not match.
        assert!(recipe.assemble(&[&blank, &blank]).is_none());
        assert!(recipe.assemble(&[&source, &source]).is_none());
        // Another colour is not in the recipe's ingredient.
        let other = ItemStack::new("minecraft:red_banner", 1);
        assert!(recipe.assemble(&[&source, &other]).is_none());
    }
    #[test]
    fn shield_decoration_needs_a_bare_shield() {
        let recipe = Special::ShieldDecoration {
            banner: ingredient(&["minecraft:red_banner"]),
            target: ingredient(&["minecraft:shield"]),
            result: "minecraft:shield".into(),
        };
        let banner = patterned("minecraft:red_banner");
        let shield = ItemStack::new("minecraft:shield", 1);
        let out = recipe.assemble(&[&banner, &shield]).expect("decorated");
        let data = out.data().unwrap();
        assert_eq!(data.base_color.as_deref(), Some("red"));
        assert_eq!(data.banner_patterns.len(), 1);
        // A shield that already has patterns cannot be decorated again.
        assert!(recipe.assemble(&[&banner, &out]).is_none());
    }
}
