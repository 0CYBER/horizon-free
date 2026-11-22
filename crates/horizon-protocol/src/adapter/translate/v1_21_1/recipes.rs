//! RecipeHolder/Ingredient packets before 1.21.2 introduced recipe displays.
//! The server's recipe keys remain authoritative; display ids are connection-local aliases.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::{
    Error, Result,
    adapter::{
        ServerVersion,
        ids::{self, Way},
        pinned_ids::item,
        session::Session,
    },
    buf::{Codec, Reader, Writer},
    ids::play::{clientbound as cb, serverbound as sb},
    item::{Slot, Stack},
    packets::recipes::{
        HolderSet, PlaceGhostRecipe, PlaceRecipe, RecipeBookAdd, RecipeBookEntry, RecipeBookRemove,
        RecipeBookSeenRecipe, RecipeBookSettings, RecipeDisplay, SlotDisplay, TrimPattern,
    },
};

const MAX_RECIPES: usize = 65_536;

#[derive(Clone)]
struct Recipe {
    id: i32,
    display: Option<RecipeDisplay>,
    group: i32,
    category: i32,
    requirements: Vec<HolderSet>,
    notify: bool,
    properties: Vec<(&'static str, Vec<i32>)>,
    stonecutting: Option<(HolderSet, SlotDisplay)>,
}

#[derive(Clone, Default)]
pub(crate) struct Recipes {
    recipes: HashMap<String, Recipe>,
    keys: Vec<String>,
    display_ids: HashMap<String, i32>,
    groups: HashMap<(i32, String), i32>,
    unlocked: HashSet<String>,
    highlighted: HashSet<String>,
    received_book: bool,
}

#[derive(Clone)]
struct Ingredient(Vec<Stack>);
impl Ingredient {
    fn read(r: &mut Reader) -> Result<Self> {
        let n = r.count(MAX_RECIPES)?;
        let mut stacks = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            if let Some(stack) = Slot::read(r)? {
                stacks.push(stack);
            }
        }
        Ok(Self(stacks))
    }
    fn items(&self) -> Vec<i32> {
        self.0
            .iter()
            .map(|s| s.item)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    fn requirement(&self) -> HolderSet {
        HolderSet::Items(self.items())
    }
    fn display(&self) -> SlotDisplay {
        let mut list: Vec<_> = self.0.iter().cloned().map(SlotDisplay::ItemStack).collect();
        match list.len() {
            0 => SlotDisplay::Empty,
            1 => list.remove(0),
            _ => SlotDisplay::Composite(list),
        }
    }
}

fn result(r: &mut Reader) -> Result<SlotDisplay> {
    Slot::read(r)?
        .map(SlotDisplay::ItemStack)
        .ok_or_else(|| Error::new("empty legacy recipe result"))
}

impl Recipes {
    fn id(&mut self, key: &str) -> Result<i32> {
        if let Some(id) = self.display_ids.get(key) {
            return Ok(*id);
        }
        if self.keys.len() >= MAX_RECIPES {
            return Err(Error::new("too many legacy recipe display ids"));
        }
        let id = self.keys.len() as i32;
        self.keys.push(key.to_owned());
        self.display_ids.insert(key.to_owned(), id);
        Ok(id)
    }
    fn group(&mut self, category: i32, group: String) -> Result<i32> {
        if group.is_empty() {
            return Ok(0);
        }
        let next = i32::try_from(self.groups.len() + 1)
            .map_err(|_| Error::new("too many recipe groups"))?;
        Ok(*self.groups.entry((category, group)).or_insert(next))
    }
    fn parse(
        &mut self,
        r: &mut Reader,
        serializer: i32,
        id: i32,
        trims: &HashMap<i32, i32>,
        width_first: bool,
    ) -> Result<Recipe> {
        // RecipeSerializer's 1.21/1.21.1 registration order from the vanilla report.
        let mut recipe = Recipe {
            id,
            display: None,
            group: 0,
            category: 3,
            requirements: Vec::new(),
            notify: true,
            properties: Vec::new(),
            stonecutting: None,
        };
        let mut ingredients = Vec::new();
        let mut group = String::new();
        match serializer {
            0 | 1 => {
                let dimensions = if serializer == 0 && width_first {
                    Some((r.varint()?, r.varint()?))
                } else {
                    None
                };
                group = r.string(32767)?;
                recipe.category = r.varint()?;
                if !(0..=3).contains(&recipe.category) {
                    return Err(Error::new("invalid legacy crafting category"));
                }
                let (width, height, count) = if serializer == 0 {
                    let (width, height) = match dimensions {
                        Some(size) => size,
                        None => (r.varint()?, r.varint()?),
                    };
                    if !(1..=3).contains(&width) || !(1..=3).contains(&height) {
                        return Err(Error::new("invalid legacy shaped recipe dimensions"));
                    }
                    (width, height, (width * height) as usize)
                } else {
                    (0, 0, r.count(9)?)
                };
                for _ in 0..count {
                    ingredients.push(Ingredient::read(r)?);
                }
                let result = result(r)?;
                let station = SlotDisplay::Item(item::CRAFTING_TABLE);
                recipe.display = Some(if serializer == 0 {
                    recipe.notify = r.bool()?;
                    RecipeDisplay::Shaped {
                        width,
                        height,
                        ingredients: ingredients.iter().map(Ingredient::display).collect(),
                        result,
                        station,
                    }
                } else {
                    RecipeDisplay::Shapeless {
                        ingredients: ingredients.iter().map(Ingredient::display).collect(),
                        result,
                        station,
                    }
                });
            }
            2..=14 | 22 => {
                // Vanilla's special crafting recipes have no fixed result/display in 1.21.2 either.
                recipe.category = r.varint()?;
                if !(0..=3).contains(&recipe.category) {
                    return Err(Error::new("invalid special recipe category"));
                }
                recipe.notify = false;
            }
            15..=18 => {
                group = r.string(32767)?;
                let category = r.varint()?;
                if !(0..=2).contains(&category) {
                    return Err(Error::new("invalid legacy cooking category"));
                }
                let ingredient = Ingredient::read(r)?;
                let result = result(r)?;
                let experience = r.f32()?;
                let duration = r.varint()?;
                let (station, property, book) = match serializer {
                    15 => (item::FURNACE, "furnace_input", 4 + category),
                    16 => (
                        item::BLAST_FURNACE,
                        "blast_furnace_input",
                        if category == 1 { 7 } else { 8 },
                    ),
                    17 => (item::SMOKER, "smoker_input", 9),
                    _ => (item::CAMPFIRE, "campfire_input", 12),
                };
                recipe.category = book;
                recipe.properties.push((property, ingredient.items()));
                recipe.display = Some(RecipeDisplay::Furnace {
                    ingredient: ingredient.display(),
                    fuel: SlotDisplay::AnyFuel,
                    result,
                    station: SlotDisplay::Item(station),
                    duration,
                    experience,
                });
                ingredients.push(ingredient);
            }
            19 => {
                group = r.string(32767)?;
                let ingredient = Ingredient::read(r)?;
                let result = result(r)?;
                recipe.category = 10;
                recipe.stonecutting = Some((ingredient.requirement(), result.clone()));
                recipe.display = Some(RecipeDisplay::Stonecutter {
                    input: ingredient.display(),
                    result,
                    station: SlotDisplay::Item(item::STONECUTTER),
                });
                ingredients.push(ingredient);
            }
            20 | 21 => {
                let template = Ingredient::read(r)?;
                let base = Ingredient::read(r)?;
                let addition = Ingredient::read(r)?;
                let result = if serializer == 20 {
                    result(r)?
                } else {
                    let mut patterns = BTreeSet::new();
                    for item in template.items() {
                        if let Some(pattern) = trims.get(&item) {
                            patterns.insert(*pattern);
                        }
                    }
                    // A template outside the synced/known set has no fabricated trim result.
                    let (base_display, addition_display) = (base.display(), addition.display());
                    // Every pattern copies both ingredients' displays: an old server's ingredient
                    // of 65 536 stacks made each 4-byte stack about 18 copies of 80-byte nodes.
                    let each = base_display
                        .weight()
                        .saturating_add(addition_display.weight());
                    if patterns.len().saturating_mul(each)
                        > crate::packets::recipes::MAX_TRIM_PREVIEW
                    {
                        return Err(Error::new("legacy trim preview too large"));
                    }
                    SlotDisplay::Composite(
                        patterns
                            .into_iter()
                            .map(|pattern| SlotDisplay::SmithingTrim {
                                base: Box::new(base_display.clone()),
                                material: Box::new(addition_display.clone()),
                                pattern: TrimPattern::Id(pattern),
                            })
                            .collect(),
                    )
                };
                recipe.category = 11;
                for (property, ingredient) in [
                    ("smithing_template", &template),
                    ("smithing_base", &base),
                    ("smithing_addition", &addition),
                ] {
                    recipe.properties.push((property, ingredient.items()));
                }
                recipe.display = Some(RecipeDisplay::Smithing {
                    template: template.display(),
                    base: base.display(),
                    addition: addition.display(),
                    result,
                    station: SlotDisplay::Item(item::SMITHING_TABLE),
                });
                ingredients.extend([template, base, addition]);
            }
            _ => {
                return Err(Error::new(format!(
                    "unknown legacy recipe serializer {serializer}"
                )));
            }
        }
        recipe.requirements = ingredients
            .iter()
            .filter(|i| !i.0.is_empty())
            .map(Ingredient::requirement)
            .collect();
        recipe.group = self.group(recipe.category, group)?;
        Ok(recipe)
    }
    fn entry(&self, key: &str, flags: i8) -> Option<RecipeBookEntry> {
        let r = self.recipes.get(key)?;
        Some(RecipeBookEntry {
            id: r.id,
            display: r.display.clone()?,
            group: r.group,
            category: r.category,
            requirements: Some(r.requirements.clone()),
            flags,
        })
    }
    fn book(&self, keys: &[String], replace: bool, added: bool) -> RecipeBookAdd {
        RecipeBookAdd {
            entries: keys
                .iter()
                .filter_map(|key| {
                    let recipe = self.recipes.get(key)?;
                    let flags = i8::from(added && recipe.notify)
                        | if self.highlighted.contains(key) { 2 } else { 0 };
                    self.entry(key, flags)
                })
                .collect(),
            replace,
        }
    }
    fn current_book(&self) -> RecipeBookAdd {
        let mut keys: Vec<_> = self.unlocked.iter().cloned().collect();
        keys.sort_unstable();
        self.book(&keys, true, false)
    }
    fn key(&self, id: i32) -> Option<&str> {
        let key = self.keys.get(usize::try_from(id).ok()?)?;
        self.recipes.get(key)?.display.as_ref()?;
        Some(key)
    }
    fn properties(&self) -> Vec<u8> {
        const NAMES: &[&str] = &[
            "smithing_addition",
            "smithing_base",
            "smithing_template",
            "furnace_input",
            "blast_furnace_input",
            "smoker_input",
            "campfire_input",
            "brewing_input",
            "brewing_reagent",
        ];
        let mut sets: BTreeMap<&str, BTreeSet<i32>> =
            NAMES.iter().map(|name| (*name, BTreeSet::new())).collect();
        let mut keys: Vec<_> = self.recipes.keys().collect();
        keys.sort_unstable();
        for recipe in self.recipes.values() {
            for (name, items) in &recipe.properties {
                sets.entry(name).or_default().extend(items);
            }
        }
        let mut w = Writer::new();
        w.varint(sets.len() as i32);
        for (name, items) in sets {
            w.string(&format!("minecraft:{name}"));
            w.varint(items.len() as i32);
            for item in items {
                w.varint(item);
            }
        }
        let stonecutting: Vec<_> = keys
            .into_iter()
            .filter_map(|key| self.recipes[key].stonecutting.as_ref())
            .collect();
        w.varint(stonecutting.len() as i32);
        for (ingredient, result) in stonecutting {
            ingredient.write(&mut w);
            result.write(&mut w);
        }
        w.data
    }
}

fn encode<T: Codec<T>>(value: &T) -> Vec<u8> {
    let mut w = Writer::new();
    T::write(value, &mut w);
    w.data
}
fn finish(r: &Reader) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing legacy recipe data"))
    }
}

/// Returns canonical pinned bytes; callers must not decode these again with older registry ids.
pub(crate) fn clientbound(session: &Session, id: i32, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut r = Reader::new(payload);
    if id == cb::UPDATE_RECIPES {
        let trims = session.trim_patterns.lock().unwrap().clone();
        let mut state = session.legacy_recipes.lock().unwrap().clone();
        state.recipes.clear();
        {
            let _up = ids::renumbering(session.version.ids().unwrap(), Way::Up);
            let n = r.count(MAX_RECIPES)?;
            for _ in 0..n {
                let legacy_serializer = if ids::quirks().legacy_item_nbt {
                    let name = r.string(32767)?;
                    let names = if session.version.at_most(ServerVersion::V1_20_2) {
                        crate::adapter::v1_20_2_ids::RECIPE_SERIALIZER_NAMES
                    } else {
                        crate::adapter::v1_20_3_ids::RECIPE_SERIALIZER_NAMES
                    };
                    Some(
                        names
                            .iter()
                            .position(|candidate| *candidate == name)
                            .ok_or_else(|| Error::new("unknown legacy recipe serializer"))?
                            as i32,
                    )
                } else {
                    None
                };
                let key = r.string(32767)?;
                let serializer = match legacy_serializer {
                    Some(id) => id,
                    None => r.varint()?,
                };
                let display = state.id(&key)?;
                let recipe = state.parse(
                    &mut r,
                    serializer,
                    display,
                    &trims,
                    session.version.at_most(ServerVersion::V1_20_2),
                )?;
                if state.recipes.insert(key, recipe).is_some() {
                    return Err(Error::new("duplicate legacy recipe key"));
                }
            }
            finish(&r)?;
        }
        let result = state.properties();
        if state.received_book {
            session
                .incoming
                .lock()
                .unwrap()
                .push_back((cb::RECIPE_BOOK_ADD, encode(&state.current_book())));
        }
        *session.legacy_recipes.lock().unwrap() = state;
        return Ok(Some(result));
    }
    if id == cb::PLACE_GHOST_RECIPE {
        let container = i32::from(r.u8()?);
        let key = r.string(32767)?;
        finish(&r)?;
        let state = session.legacy_recipes.lock().unwrap();
        let display = state
            .recipes
            .get(&key)
            .and_then(|r| r.display.clone())
            .ok_or_else(|| Error::new("legacy ghost recipe has no known display"))?;
        return Ok(Some(encode(&PlaceGhostRecipe { container, display })));
    }
    Ok(None)
}

/// The old combined recipe packet becomes settings followed by a canonical add/remove packet.
pub(crate) fn recipe_book(session: &Session, payload: &[u8]) -> Result<(i32, Vec<u8>)> {
    let mut r = Reader::new(payload);
    let action = r.varint()?;
    if !(0..=2).contains(&action) {
        return Err(Error::new("invalid legacy recipe-book action"));
    }
    let settings = RecipeBookSettings::read(&mut r)?;
    let n = r.count(MAX_RECIPES)?;
    let mut keys = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        keys.push(r.string(32767)?);
    }
    let mut highlights = HashSet::new();
    if action == 0 {
        let n = r.count(MAX_RECIPES)?;
        for _ in 0..n {
            highlights.insert(r.string(32767)?);
        }
    }
    finish(&r)?;
    let mut state = session.legacy_recipes.lock().unwrap();
    if action == 1
        && state
            .unlocked
            .iter()
            .cloned()
            .chain(keys.iter().cloned())
            .collect::<HashSet<_>>()
            .len()
            > MAX_RECIPES
    {
        return Err(Error::new("too many unlocked legacy recipes"));
    }
    state.received_book = true;
    let packet = if action == 2 {
        let ids = keys
            .iter()
            .filter_map(|key| state.recipes.get(key))
            .filter(|r| r.display.is_some())
            .map(|r| r.id)
            .collect();
        for key in &keys {
            state.unlocked.remove(key);
            state.highlighted.remove(key);
        }
        (cb::RECIPE_BOOK_REMOVE, encode(&RecipeBookRemove { ids }))
    } else {
        if action == 0 {
            state.unlocked.clear();
            state.highlighted = highlights;
        } else {
            state.highlighted.extend(keys.iter().cloned());
        }
        state.unlocked.extend(keys.iter().cloned());
        (
            cb::RECIPE_BOOK_ADD,
            encode(&state.book(&keys, action == 0, action == 1)),
        )
    };
    session.incoming.lock().unwrap().push_back(packet);
    Ok((cb::RECIPE_BOOK_SETTINGS, encode(&settings)))
}

/// Call only for PLACE_RECIPE / RECIPE_BOOK_SEEN_RECIPE. `None` drops a stale display id.
pub(crate) fn serverbound(session: &Session, id: i32, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut r = Reader::new(payload);
    let (display, container, use_max) = match id {
        sb::PLACE_RECIPE => {
            let p = PlaceRecipe::read(&mut r)?;
            let container = i8::try_from(p.container)
                .map_err(|_| Error::new("legacy recipe container is out of range"))?;
            (p.recipe, Some(container), p.use_max)
        }
        sb::RECIPE_BOOK_SEEN_RECIPE => (RecipeBookSeenRecipe::read(&mut r)?.recipe, None, false),
        _ => return Ok(None),
    };
    finish(&r)?;
    let state = session.legacy_recipes.lock().unwrap();
    let Some(key) = state.key(display) else {
        return Ok(None);
    };
    let mut w = Writer::new();
    if let Some(container) = container {
        w.i8(container);
    }
    w.string(key);
    if container.is_some() {
        w.bool(use_max);
    }
    Ok(Some(w.data))
}

#[cfg(test)]
mod tests;
