//! The recipe book: what the book shows for each menu (`RecipeBookType`
//! tabs, `RecipeBookCategories`, groups), which recipes the player can craft
//! with the items at hand (`StackedItemContents.canCraft` and the biggest
//! craftable stack), where a recipe goes in a grid (`PlaceRecipeHelper`) and
//! the book's saved settings.
use crate::{
    crafting::{Ingredient, Recipe, Recipes},
    items::{ItemStack, Items},
};
use horizon_world::nbt::Tag;
use std::collections::{BTreeSet, HashMap};

/// `RecipeBookType`: the menus with a recipe book and their own settings.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BookType {
    Crafting,
    Furnace,
    BlastFurnace,
    Smoker,
}
impl BookType {
    pub const ALL: [Self; 4] = [
        Self::Crafting,
        Self::Furnace,
        Self::BlastFurnace,
        Self::Smoker,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
    /// `RecipeBookSettings` keys in the player's `recipeBook` compound.
    fn keys(self) -> (&'static str, &'static str) {
        match self {
            Self::Crafting => ("isGuiOpen", "isFilteringCraftable"),
            Self::Furnace => ("isFurnaceGuiOpen", "isFurnaceFilteringCraftable"),
            Self::BlastFurnace => (
                "isBlastingFurnaceGuiOpen",
                "isBlastingFurnaceFilteringCraftable",
            ),
            Self::Smoker => ("isSmokerGuiOpen", "isSmokerFilteringCraftable"),
        }
    }
    /// The tab buttons of the book's screen, search first.
    pub fn tabs(self) -> Vec<Tab> {
        use Category::*;
        let categories: &[Category] = match self {
            Self::Crafting => &[
                CraftingEquipment,
                CraftingBuilding,
                CraftingMisc,
                CraftingRedstone,
            ],
            Self::Furnace => &[FurnaceFood, FurnaceBlocks, FurnaceMisc],
            Self::BlastFurnace => &[BlastBlocks, BlastMisc],
            Self::Smoker => &[SmokerFood],
        };
        std::iter::once(Tab::Search)
            .chain(categories.iter().map(|c| Tab::Category(*c)))
            .collect()
    }
    /// The categories the search tab lists, in `SearchRecipeBookCategory` order.
    fn search_categories(self) -> &'static [Category] {
        use Category::*;
        match self {
            Self::Crafting => &[
                CraftingEquipment,
                CraftingBuilding,
                CraftingMisc,
                CraftingRedstone,
            ],
            Self::Furnace => &[FurnaceFood, FurnaceBlocks, FurnaceMisc],
            Self::BlastFurnace => &[BlastBlocks, BlastMisc],
            Self::Smoker => &[SmokerFood],
        }
    }
}

/// `RecipeBookCategories` of the four menus' recipes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Category {
    CraftingBuilding,
    CraftingRedstone,
    CraftingEquipment,
    CraftingMisc,
    FurnaceFood,
    FurnaceBlocks,
    FurnaceMisc,
    BlastBlocks,
    BlastMisc,
    SmokerFood,
}
/// A tab button: the search tab or one category.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tab {
    Search,
    Category(Category),
}
impl Tab {
    /// The tab's icon items (`TabInfo`).
    pub fn icons(self) -> (&'static str, Option<&'static str>) {
        use Category::*;
        match self {
            Self::Search => ("minecraft:compass", None),
            Self::Category(c) => match c {
                CraftingEquipment => ("minecraft:iron_axe", Some("minecraft:golden_sword")),
                CraftingBuilding => ("minecraft:bricks", None),
                CraftingMisc => ("minecraft:lava_bucket", Some("minecraft:apple")),
                CraftingRedstone => ("minecraft:redstone", None),
                FurnaceFood | SmokerFood => ("minecraft:porkchop", None),
                FurnaceBlocks => ("minecraft:stone", None),
                FurnaceMisc => ("minecraft:lava_bucket", Some("minecraft:emerald")),
                BlastBlocks => ("minecraft:redstone_ore", None),
                BlastMisc => ("minecraft:iron_shovel", Some("minecraft:golden_leggings")),
            },
        }
    }
}

/// `RecipeDisplay`: how a recipe is laid out.
#[derive(Clone, Debug)]
pub enum Display {
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<Ingredient>>,
    },
    Shapeless(Vec<Ingredient>),
    /// A smelting-type recipe: its input; the fuel is any fuel.
    Furnace(Ingredient),
}
/// One recipe of the book (`RecipeDisplayEntry`).
#[derive(Clone, Debug)]
pub struct Entry {
    /// The recipe id, `minecraft:name`.
    pub id: String,
    pub category: Category,
    pub group: Option<String>,
    pub display: Display,
    pub result: ItemStack,
}
impl Entry {
    /// The ingredients that must be matched to craft it, in order.
    pub fn requirements(&self) -> Vec<&Ingredient> {
        match &self.display {
            Display::Shaped { cells, .. } => cells.iter().flatten().collect(),
            Display::Shapeless(list) => list.iter().collect(),
            Display::Furnace(i) => vec![i],
        }
    }
    /// `CraftingRecipeBookComponent.canDisplay` for a grid.
    pub fn fits(&self, grid_width: usize, grid_height: usize) -> bool {
        match &self.display {
            Display::Shaped { width, height, .. } => grid_width >= *width && grid_height >= *height,
            Display::Shapeless(list) => grid_width * grid_height >= list.len(),
            Display::Furnace(_) => true,
        }
    }
    /// The size the recipe takes in the grid (`PlaceRecipeHelper`).
    pub fn size_in(&self, grid_width: usize, grid_height: usize) -> (usize, usize) {
        match &self.display {
            Display::Shaped { width, height, .. } => (*width, *height),
            _ => (grid_width, grid_height),
        }
    }
    /// The ingredient of each grid cell the recipe fills, with its grid index
    /// (the ghost recipe and the placement use the same layout).
    pub fn grid_cells(&self, grid_width: usize, grid_height: usize) -> Vec<(Option<usize>, usize)> {
        let (w, h) = self.size_in(grid_width, grid_height);
        // One entry per recipe cell: the index of its ingredient, if any.
        let entries: Vec<Option<usize>> = match &self.display {
            Display::Shaped { cells, .. } => {
                let mut next = 0;
                cells
                    .iter()
                    .map(|c| {
                        c.as_ref().map(|_| {
                            next += 1;
                            next - 1
                        })
                    })
                    .collect()
            }
            Display::Shapeless(list) => (0..list.len()).map(Some).collect(),
            Display::Furnace(_) => vec![Some(0)],
        };
        place_cells(grid_width, grid_height, w, h, &entries)
    }
}

/// `PlaceRecipeHelper.placeRecipe`: the grid index each entry goes to; a
/// recipe smaller than half the grid is centred.
pub fn place_cells<T: Clone>(
    grid_width: usize,
    grid_height: usize,
    recipe_width: usize,
    recipe_height: usize,
    entries: &[T],
) -> Vec<(T, usize)> {
    let mut out = Vec::new();
    let mut iter = entries.iter().peekable();
    let mut index = 0usize;
    let mut y = 0;
    while y < grid_height {
        let center_rows = (recipe_height as f32) < grid_height as f32 / 2.0;
        let start_row = (grid_height as f32 / 2.0 - recipe_height as f32 / 2.0).floor() as usize;
        if center_rows && start_row > y {
            index += grid_width;
            y += 1;
        }
        for x in 0..grid_width {
            if iter.peek().is_none() {
                return out;
            }
            let center_cols = (recipe_width as f32) < grid_width as f32 / 2.0;
            let start_col = (grid_width as f32 / 2.0 - recipe_width as f32 / 2.0).floor() as usize;
            let mut total = recipe_width;
            let mut add = x < recipe_width;
            if center_cols {
                total = start_col + recipe_width;
                add = start_col <= x && x < start_col + recipe_width;
            }
            if add {
                if let Some(item) = iter.next() {
                    out.push((item.clone(), index));
                }
            } else if total == x {
                index += grid_width - x;
                break;
            }
            index += 1;
        }
        y += 1;
    }
    out
}

/// A group of recipes shown as one button (`RecipeCollection`).
#[derive(Clone, Debug, Default)]
pub struct Collection {
    /// Indices into `Book::entries`.
    pub entries: Vec<usize>,
}

/// Every recipe the menus' books can show.
#[derive(Default)]
pub struct Book {
    pub entries: Vec<Entry>,
    /// Item id -> registry position, to order ingredient choices.
    pub order: HashMap<String, usize>,
}
fn crafting_category(name: &str) -> Category {
    match name {
        "building" => Category::CraftingBuilding,
        "redstone" => Category::CraftingRedstone,
        "equipment" => Category::CraftingEquipment,
        _ => Category::CraftingMisc,
    }
}
impl Book {
    pub fn build(recipes: &Recipes, items: &Items) -> Self {
        let mut entries = Vec::new();
        let group = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        for (name, recipe) in &recipes.crafting {
            let (category, grp) = recipes.meta.get(name).cloned().unwrap_or_default();
            let (display, result) = match recipe {
                Recipe::Shaped {
                    width,
                    height,
                    cells,
                    result,
                } => (
                    Display::Shaped {
                        width: *width,
                        height: *height,
                        cells: cells.clone(),
                    },
                    result,
                ),
                Recipe::Shapeless {
                    ingredients,
                    result,
                } => (Display::Shapeless(ingredients.clone()), result),
                Recipe::Special(_) => continue,
            };
            entries.push(Entry {
                id: format!("minecraft:{name}"),
                category: crafting_category(&category),
                group: group(&grp),
                display,
                result: result.clone(),
            });
        }
        for c in &recipes.cooking {
            let category = match (c.kind.as_str(), c.category.as_str()) {
                ("minecraft:smelting", "food") => Category::FurnaceFood,
                ("minecraft:smelting", "blocks") => Category::FurnaceBlocks,
                ("minecraft:smelting", _) => Category::FurnaceMisc,
                ("minecraft:blasting", "blocks") => Category::BlastBlocks,
                ("minecraft:blasting", _) => Category::BlastMisc,
                ("minecraft:smoking", _) => Category::SmokerFood,
                _ => continue,
            };
            entries.push(Entry {
                id: format!("minecraft:{}", c.id),
                category,
                group: group(&c.group),
                display: Display::Furnace(c.ingredient.clone()),
                result: c.result.clone(),
            });
        }
        let order = items
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.to_string(), i))
            .collect();
        Self { entries, order }
    }
    /// The unlocked recipes of one tab as collections (`ClientRecipeBook.rebuildCollections`).
    pub fn collections(
        &self,
        unlocked: &BTreeSet<String>,
        book: BookType,
        tab: Tab,
    ) -> Vec<Collection> {
        let categories: Vec<Category> = match tab {
            Tab::Search => book.search_categories().to_vec(),
            Tab::Category(c) => vec![c],
        };
        let mut out = Vec::new();
        for category in categories {
            let mut by_group: HashMap<&str, usize> = HashMap::new();
            let start = out.len();
            for (i, e) in self.entries.iter().enumerate() {
                if e.category != category || !unlocked.contains(&e.id) {
                    continue;
                }
                match e.group.as_deref().and_then(|g| by_group.get(g).copied()) {
                    Some(c) => {
                        let c: usize = c;
                        let collection: &mut Collection = &mut out[start + c];
                        collection.entries.push(i);
                    }
                    None => {
                        if let Some(g) = e.group.as_deref() {
                            by_group.insert(g, out.len() - start);
                        }
                        out.push(Collection { entries: vec![i] });
                    }
                }
            }
        }
        out
    }
    /// Which book a recipe id belongs to.
    pub fn book_of(&self, id: &str) -> Option<BookType> {
        let e = self.entries.iter().find(|e| e.id == id)?;
        Some(match e.category {
            Category::CraftingBuilding
            | Category::CraftingRedstone
            | Category::CraftingEquipment
            | Category::CraftingMisc => BookType::Crafting,
            Category::FurnaceFood | Category::FurnaceBlocks | Category::FurnaceMisc => {
                BookType::Furnace
            }
            Category::BlastBlocks | Category::BlastMisc => BookType::BlastFurnace,
            Category::SmokerFood => BookType::Smoker,
        })
    }
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
    /// `ingredient`'s items in registry order (the cycling order of ghost slots).
    pub fn items_of(&self, ingredient: &Ingredient) -> Vec<String> {
        let mut items: Vec<String> = ingredient.0.iter().cloned().collect();
        items.sort_by_key(|i| (self.order.get(i).copied().unwrap_or(usize::MAX), i.clone()));
        items
    }
    /// `RecipeCollection` search: the result's name or registry path, or
    /// `@namespace`.
    pub fn matches_search(&self, entry: &Entry, query: &str, name: &str) -> bool {
        let query = query.to_lowercase();
        if let Some(ns) = query.strip_prefix('@') {
            return entry
                .result
                .item
                .split(':')
                .next()
                .is_some_and(|n| n.starts_with(ns));
        }
        name.to_lowercase().contains(&query)
            || entry
                .result
                .item
                .trim_start_matches("minecraft:")
                .contains(&query)
    }
}

/// `StackedItemContents`: simple stacks (undamaged, unenchanted, unnamed) by item.
#[derive(Clone, Debug, Default)]
pub struct Available {
    pub counts: HashMap<String, u32>,
}
impl Available {
    /// `accountSimpleStack`.
    pub fn add(&mut self, stack: &ItemStack) {
        if stack.count > 0
            && stack.damage == 0
            && stack.enchantments().is_empty()
            && stack.custom_name().is_none()
        {
            *self.counts.entry(stack.item.to_string()).or_default() += stack.count;
        }
    }
    pub fn from_stacks<'a>(stacks: impl IntoIterator<Item = &'a ItemStack>) -> Self {
        let mut a = Self::default();
        for s in stacks {
            a.add(s);
        }
        a
    }
    /// `StackedItemContents.canCraft(ingredients, null)`.
    pub fn can_craft(&self, requirements: &[&Ingredient]) -> bool {
        !requirements.is_empty() && pick(requirements, &self.counts, 1, &HashMap::new()).is_some()
    }
    /// `canCraft(recipe, amount, output)`: the item each ingredient uses when
    /// `amount` sets can be made, preferring earlier registry items.
    pub fn pick(
        &self,
        requirements: &[&Ingredient],
        amount: u32,
        order: &HashMap<String, usize>,
    ) -> Option<Vec<String>> {
        pick(requirements, &self.counts, amount, order)
    }
    /// `getBiggestCraftableStack`: how many sets can be made, up to 99.
    pub fn biggest_stack(&self, requirements: &[&Ingredient]) -> u32 {
        if !self.can_craft(requirements) {
            return 0;
        }
        let none = HashMap::new();
        let (mut lo, mut hi) = (1u32, 99u32);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if pick(requirements, &self.counts, mid, &none).is_some() {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }
}
/// Bipartite assignment of one item to each ingredient where an item can
/// serve `count / amount` ingredients.
fn pick(
    requirements: &[&Ingredient],
    counts: &HashMap<String, u32>,
    amount: u32,
    order: &HashMap<String, usize>,
) -> Option<Vec<String>> {
    let amount = amount.max(1);
    let capacity = |item: &str| counts.get(item).copied().unwrap_or(0) / amount;
    let candidates: Vec<Vec<String>> = requirements
        .iter()
        .map(|ing| {
            let mut v: Vec<String> = ing.0.iter().filter(|i| capacity(i) > 0).cloned().collect();
            v.sort_by_key(|i| (order.get(i).copied().unwrap_or(usize::MAX), i.clone()));
            v
        })
        .collect();
    let mut used: HashMap<String, Vec<usize>> = HashMap::new();
    fn assign(
        i: usize,
        candidates: &[Vec<String>],
        used: &mut HashMap<String, Vec<usize>>,
        visited: &mut Vec<String>,
        capacity: &dyn Fn(&str) -> u32,
    ) -> bool {
        for item in &candidates[i] {
            if visited.contains(item) {
                continue;
            }
            visited.push(item.clone());
            let holders = used.get(item).cloned().unwrap_or_default();
            if (holders.len() as u32) < capacity(item) {
                used.entry(item.clone()).or_default().push(i);
                return true;
            }
            for j in holders {
                if assign(j, candidates, used, visited, capacity) {
                    // j moved elsewhere: drop it from this item and take its place.
                    if let Some(list) = used.get_mut(item) {
                        list.retain(|x| *x != j);
                        list.push(i);
                    }
                    return true;
                }
            }
        }
        false
    }
    for i in 0..requirements.len() {
        let mut visited = Vec::new();
        if !assign(i, &candidates, &mut used, &mut visited, &capacity) {
            return None;
        }
    }
    let mut result = vec![String::new(); requirements.len()];
    for (item, holders) in used {
        for i in holders {
            result[i] = item.clone();
        }
    }
    Some(result)
}

/// The recipe book's saved state: the `recipeBook` compound of the player.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// `isGuiOpen`-style flags per `BookType`.
    pub open: [bool; 4],
    pub filtering: [bool; 4],
    /// Unlocked recipe ids (`recipes`).
    pub recipes: Vec<String>,
}
impl Settings {
    pub fn read(tag: Option<&Tag>) -> Self {
        let mut s = Self::default();
        let Some(tag) = tag else { return s };
        let flag = |k: &str| matches!(tag.get(k), Some(Tag::Byte(b)) if *b != 0);
        for b in BookType::ALL {
            let (open, filter) = b.keys();
            s.open[b.index()] = flag(open);
            s.filtering[b.index()] = flag(filter);
        }
        if let Some(Tag::List(list)) = tag.get("recipes") {
            s.recipes = list
                .iter()
                .filter_map(|t| match t {
                    Tag::String(id) => Some(id.to_string_lossy()),
                    _ => None,
                })
                .collect();
        }
        s
    }
    /// The compound written to the player file.
    pub fn write(&self) -> Tag {
        let mut entries: Vec<(&str, Tag)> = Vec::new();
        for b in BookType::ALL {
            let (open, filter) = b.keys();
            // The codec omits settings at their default (false).
            if self.open[b.index()] {
                entries.push((open, Tag::Byte(1)));
            }
            if self.filtering[b.index()] {
                entries.push((filter, Tag::Byte(1)));
            }
        }
        let list = |ids: &[String]| {
            Tag::List(ids.iter().map(|i| Tag::String(i.as_str().into())).collect())
        };
        entries.push(("recipes", list(&self.recipes)));
        Tag::compound(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ing(items: &[&str]) -> Ingredient {
        Ingredient(items.iter().map(|s| (*s).to_owned()).collect())
    }
    fn have(items: &[(&str, u32)]) -> Available {
        Available {
            counts: items.iter().map(|(i, n)| ((*i).to_owned(), *n)).collect(),
        }
    }
    #[test]
    fn small_recipes_are_centred_in_the_grid() {
        // One ingredient in a 3x3 grid goes to the middle cell.
        assert_eq!(place_cells(3, 3, 1, 1, &['a']), vec![('a', 4)]);
        // A 2x2 recipe fills the top-left corner.
        let cells: Vec<usize> = place_cells(3, 3, 2, 2, &['a', 'b', 'c', 'd'])
            .into_iter()
            .map(|(_, i)| i)
            .collect();
        assert_eq!(cells, vec![0, 1, 3, 4]);
        // A one-row recipe sits on the middle row.
        let row: Vec<usize> = place_cells(3, 3, 3, 1, &['a', 'b', 'c'])
            .into_iter()
            .map(|(_, i)| i)
            .collect();
        assert_eq!(row, vec![3, 4, 5]);
        // The 2x2 grid is never centred.
        let small: Vec<usize> = place_cells(2, 2, 1, 1, &['a'])
            .into_iter()
            .map(|(_, i)| i)
            .collect();
        assert_eq!(small, vec![0]);
    }
    #[test]
    fn a_single_item_can_fill_several_ingredients() {
        let planks = ing(&["minecraft:oak_planks", "minecraft:birch_planks"]);
        let reqs = [&planks, &planks, &planks, &planks];
        let a = have(&[("minecraft:oak_planks", 2), ("minecraft:birch_planks", 2)]);
        assert!(a.can_craft(&reqs));
        let none = HashMap::new();
        // Two sets need 8 planks, but only 4 are there.
        assert!(a.pick(&reqs, 2, &none).is_none());
        assert!(a.pick(&reqs, 1, &none).is_some());
        assert_eq!(a.biggest_stack(&reqs), 1);
        assert_eq!(
            have(&[("minecraft:oak_planks", 64)]).biggest_stack(&reqs),
            16
        );
    }
    #[test]
    fn the_matching_reassigns_flexible_ingredients() {
        // The first ingredient takes either item, the second only the stick:
        // one stick must not be wasted on the first.
        let any = ing(&["minecraft:stick", "minecraft:bone"]);
        let stick = ing(&["minecraft:stick"]);
        let a = have(&[("minecraft:stick", 1), ("minecraft:bone", 1)]);
        let mut order = HashMap::new();
        order.insert("minecraft:stick".to_owned(), 0);
        order.insert("minecraft:bone".to_owned(), 1);
        let picked = a.pick(&[&any, &stick], 1, &order).expect("matching");
        assert_eq!(picked, vec!["minecraft:bone", "minecraft:stick"]);
        assert!(!have(&[("minecraft:stick", 1)]).can_craft(&[&any, &stick]));
    }
    #[test]
    fn damaged_or_named_stacks_are_not_counted() {
        let mut named = ItemStack::new("minecraft:stick", 4);
        named.data_mut().custom_name = Some("x".into());
        let mut damaged = ItemStack::new("minecraft:iron_sword", 1);
        damaged.damage = 3;
        let a = Available::from_stacks([&named, &damaged, &ItemStack::new("minecraft:stick", 2)]);
        assert_eq!(a.counts.get("minecraft:stick"), Some(&2));
        assert!(!a.counts.contains_key("minecraft:iron_sword"));
    }
    #[test]
    fn collections_group_recipes_of_a_category() {
        let entry = |id: &str, group: Option<&str>, category| Entry {
            id: id.into(),
            category,
            group: group.map(str::to_owned),
            display: Display::Shapeless(vec![ing(&["minecraft:stick"])]),
            result: ItemStack::new("minecraft:stick", 1),
        };
        let book = Book {
            entries: vec![
                entry("minecraft:a", Some("planks"), Category::CraftingBuilding),
                entry("minecraft:b", Some("planks"), Category::CraftingBuilding),
                entry("minecraft:c", None, Category::CraftingBuilding),
                entry("minecraft:d", Some("planks"), Category::CraftingMisc),
            ],
            order: HashMap::new(),
        };
        let unlocked: BTreeSet<String> = ["a", "b", "c", "d"]
            .iter()
            .map(|n| format!("minecraft:{n}"))
            .collect();
        let tab = Tab::Category(Category::CraftingBuilding);
        let c = book.collections(&unlocked, BookType::Crafting, tab);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].entries, vec![0, 1]);
        // The search tab lists every category; the same group in another category is separate.
        let all = book.collections(&unlocked, BookType::Crafting, Tab::Search);
        assert_eq!(all.len(), 3);
        // Locked recipes are hidden.
        let some: BTreeSet<String> = ["minecraft:c".to_owned()].into();
        assert_eq!(book.collections(&some, BookType::Crafting, tab).len(), 1);
    }
    #[test]
    fn settings_round_trip() {
        let mut s = Settings::default();
        s.open[BookType::Furnace.index()] = true;
        s.filtering[BookType::Crafting.index()] = true;
        s.recipes = vec!["minecraft:stick".into()];
        let back = Settings::read(Some(&s.write()));
        assert_eq!(back.open, s.open);
        assert_eq!(back.filtering, s.filtering);
        assert_eq!(back.recipes, s.recipes);
    }
}
