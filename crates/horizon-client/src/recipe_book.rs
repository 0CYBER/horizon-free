//! The recipe book: what the book shows for each menu (`RecipeBookType`
//! tabs, `RecipeBookCategories`, groups), which recipes the player can craft
//! with the items at hand (`StackedItemContents.canCraft` and the biggest
//! craftable stack), where a recipe goes in a grid (`PlaceRecipeHelper`) and
//! the book's saved settings.
use crate::{
    crafting::{Ingredient, Placement, Recipe, Recipes},
    items::{ItemStack, Items},
};
use horizon_core::nbt::Tag;
use horizon_protocol::packets::recipes::{HolderSet, RecipeBookEntry, RecipeDisplay, SlotDisplay};
use horizon_world::loot::PackOverlay;
pub use mod_api::item::recipe_book::RecipeBookCategory;
use std::collections::{BTreeSet, HashMap};

mod properties;
pub use properties::ServerRecipeProperties;

mod custom_books;
pub mod menu_layout;
mod mod_index;
pub mod mod_presentation;
mod placement_codec;
pub mod placement_sync;
mod resource_id;
pub mod settings_codec;
pub mod settings_connection;
pub mod settings_sync;
pub mod slot_layout;
pub use custom_books::{BookError, BookFlags, CustomBooks, read_saved};
pub use mod_index::{DisplayLoadError, DisplayResources, ModBookIndex, ModBookPanel};

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
impl Category {
    /// The `RecipeBookCategories` registry path.
    pub fn name(self) -> &'static str {
        match self {
            Self::CraftingBuilding => "crafting_building_blocks",
            Self::CraftingRedstone => "crafting_redstone",
            Self::CraftingEquipment => "crafting_equipment",
            Self::CraftingMisc => "crafting_misc",
            Self::FurnaceFood => "furnace_food",
            Self::FurnaceBlocks => "furnace_blocks",
            Self::FurnaceMisc => "furnace_misc",
            Self::BlastBlocks => "blast_furnace_blocks",
            Self::BlastMisc => "blast_furnace_misc",
            Self::SmokerFood => "smoker_food",
        }
    }
    /// The category of a `minecraft:recipe_book_category` name; none for the
    /// ones this book has no tab for (stonecutter, smithing, campfire).
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        [
            Self::CraftingBuilding,
            Self::CraftingRedstone,
            Self::CraftingEquipment,
            Self::CraftingMisc,
            Self::FurnaceFood,
            Self::FurnaceBlocks,
            Self::FurnaceMisc,
            Self::BlastBlocks,
            Self::BlastMisc,
            Self::SmokerFood,
        ]
        .into_iter()
        .find(|c| c.name() == name)
    }
    /// The category of a pinned `minecraft:recipe_book_category` protocol id;
    /// none for the ones this book has no tab for (stonecutter, smithing, campfire).
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::CraftingBuilding),
            1 => Some(Self::CraftingRedstone),
            2 => Some(Self::CraftingEquipment),
            3 => Some(Self::CraftingMisc),
            4 => Some(Self::FurnaceFood),
            5 => Some(Self::FurnaceBlocks),
            6 => Some(Self::FurnaceMisc),
            7 => Some(Self::BlastBlocks),
            8 => Some(Self::BlastMisc),
            9 => Some(Self::SmokerFood),
            _ => None,
        }
    }
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
    /// `RecipeDisplayId`: the display's index among every recipe's displays
    /// (`Book::set_display_ids`; -1 until then).
    pub display_id: i32,
    pub category: Category,
    pub group: Option<String>,
    pub display: Display,
    pub result: ItemStack,
    /// Every stack its display can show as the result when that is more than
    /// `result` (a map copy of any map, an arrow of any potion): the search
    /// tree reads them all, the button shows the first.
    pub variants: Vec<ItemStack>,
    /// Ingredients shown as other stacks than their items (by index in
    /// `requirements`): the potions an imbued arrow's lingering potion is.
    pub stacks: Vec<(usize, Vec<ItemStack>)>,
}
impl Entry {
    /// Every stack the result display cycles through.
    pub fn results(&self) -> &[ItemStack] {
        if self.variants.is_empty() {
            std::slice::from_ref(&self.result)
        } else {
            &self.variants
        }
    }
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
    /// `Recipe.placementInfo` by recipe id: what the book's craftable test
    /// and the server's placement look for.
    pub placements: HashMap<String, Placement>,
    /// Item id -> registry position, to order ingredient choices.
    pub order: HashMap<String, usize>,
    /// The ingredients a server's entry asks for (`craftingRequirements`), by
    /// the entry's display id; none for an entry that sent none.
    pub server_requirements: HashMap<i32, Option<Vec<Ingredient>>>,
    /// The name each group of a server's entries goes by, by the server's group
    /// number (`1 + index`): the name of a book entry kept in that group, so
    /// that the entries taken from the packet join it.
    pub server_groups: HashMap<i32, String>,
    /// The display ids the server sent an entry for that this book numbers the
    /// same way: the book's own entry there is the same recipe (`same_recipe`).
    /// Only these tie a recipe's name to the number the server knows it by.
    confirmed: std::collections::BTreeSet<i32>,
    /// The confirmed display ids of each recipe in the book's order, made when
    /// first asked and dropped whenever `confirmed` changes.
    confirmed_by_recipe: std::sync::OnceLock<HashMap<String, std::sync::Arc<[i32]>>>,
}
pub(crate) fn crafting_category(name: &str) -> Category {
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
                Recipe::Mod(_) => continue,
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
                // Custom recipes are never learnable; the others show one
                // display per material count (TransmuteRecipe.display).
                Recipe::Special(special) => {
                    for shown in special.book_displays() {
                        entries.push(Entry {
                            id: crate::crafting::qualified(name),
                            display_id: -1,
                            category: crafting_category(&category),
                            group: group(&grp),
                            display: shown.display,
                            result: shown.result,
                            variants: shown.variants,
                            stacks: shown.stacks,
                        });
                    }
                    continue;
                }
            };
            entries.push(Entry {
                id: crate::crafting::qualified(name),
                display_id: -1,
                category: crafting_category(&category),
                group: group(&grp),
                display,
                result: result.create(),
                variants: Vec::new(),
                stacks: Vec::new(),
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
                id: crate::crafting::qualified(&c.id),
                display_id: -1,
                category,
                group: group(&c.group),
                display: Display::Furnace(c.ingredient.clone()),
                result: c.result.clone(),
                variants: Vec::new(),
                stacks: Vec::new(),
            });
        }
        let order = items
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.to_string(), i))
            .collect();
        let placements: HashMap<_, _> = entries
            .iter()
            .filter_map(|entry| {
                let placement = recipes.placement(&entry.id)?;
                (!placement.impossible()).then(|| (entry.id.clone(), placement))
            })
            .collect();
        entries.retain(|entry| placements.contains_key(&entry.id));
        Self {
            entries,
            placements,
            order,
            server_requirements: HashMap::new(),
            server_groups: HashMap::new(),
            ..Default::default()
        }
    }
    /// `RecipeDisplayEntry.canCraft`: the recipe's placement ingredients
    /// (`craftingRequirements`) can all be found in `stock`.
    pub fn can_craft(&self, entry: &Entry, stock: &crate::menu::place::Stacked) -> bool {
        // A server's entry: the ingredients it sent, none meaning not craftable.
        if let Some(requirements) = self.server_requirements.get(&entry.display_id) {
            return requirements
                .as_ref()
                .is_some_and(|r| stock.can_craft(&r.iter().collect::<Vec<_>>()));
        }
        self.placements.get(&entry.id).is_some_and(|p| {
            !p.impossible() && stock.can_craft(&p.ingredients.iter().collect::<Vec<_>>())
        })
    }
    /// Numbers the entries as `RecipeManager.unpackRecipeInfo` numbers the
    /// displays: every recipe in id order, each with its displays.
    pub fn set_display_ids(&mut self, index: &RecipeIndex) {
        // New numbers: no number the server sent is known to match any more.
        self.confirmed.clear();
        self.confirmed_by_recipe = Default::default();
        let mut previous: Option<(String, i32)> = None;
        for e in &mut self.entries {
            let next = match &previous {
                Some((id, last)) if *id == e.id => last + 1,
                _ => index.first_display(&e.id).unwrap_or(-1),
            };
            e.display_id = next;
            previous = Some((e.id.clone(), next));
        }
    }
    /// The entry of a display id.
    pub fn entry_of_display(&self, display: i32) -> Option<usize> {
        self.entries.iter().position(|e| e.display_id == display)
    }
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
    /// `ingredient`'s items as its display resolves them (the cycling order
    /// of ghost slots and the overlay).
    pub fn items_of(&self, ingredient: &Ingredient) -> Vec<String> {
        ingredient.shown.as_ref().map_or_else(
            || ingredient.order.clone(),
            |stacks| stacks.iter().map(|stack| stack.item.to_string()).collect(),
        )
    }
    /// `RecipeBookComponent.fillGhostRecipe` of a book type: the result and the
    /// ingredients faded into the menu's slots, in the order the component
    /// sets them. `grid` is the side of the crafting grid, `fuel_empty`
    /// whether the furnace's fuel slot is empty.
    pub fn ghost(
        &self,
        entry: &Entry,
        book_type: BookType,
        grid: usize,
        fuel_empty: bool,
        items: &crate::items::Items,
    ) -> Vec<GhostSlot> {
        let stacks = |ingredient: &Ingredient| -> Vec<ItemStack> {
            ingredient.shown.clone().unwrap_or_else(|| {
                self.items_of(ingredient)
                    .iter()
                    .map(|i| ItemStack::new(i, 1))
                    .collect()
            })
        };
        let mut out = Vec::new();
        match book_type {
            BookType::Crafting => {
                out.push(GhostSlot {
                    target: GhostTarget::Result,
                    items: entry.results().to_vec(),
                });
                let requirements = entry.requirements();
                for (ingredient, index) in entry.grid_cells(grid, grid) {
                    let Some(i) = ingredient else { continue };
                    let list = match entry.stacks.iter().find(|s| s.0 == i) {
                        Some(shown) => shown.1.clone(),
                        None => stacks(requirements[i]),
                    };
                    if !list.is_empty() {
                        out.push(GhostSlot {
                            target: GhostTarget::Grid(index),
                            items: list,
                        });
                    }
                }
            }
            _ => {
                out.push(GhostSlot {
                    target: GhostTarget::Result,
                    items: entry.results().to_vec(),
                });
                if let Display::Furnace(ingredient) = &entry.display {
                    out.push(GhostSlot {
                        target: GhostTarget::Input,
                        items: stacks(ingredient),
                    });
                    if fuel_empty {
                        // `SlotDisplay.AnyFuel`: every item with a cooking fuel
                        // component, the registry's component lookup listing the
                        // items of one component value together, the values in
                        // the order of their first item.
                        let mut groups: Vec<(String, Vec<ItemStack>)> = Vec::new();
                        for item in &items.registry_order {
                            let Some(fuel) = items.get(item).and_then(|i| i.fuel.as_ref()) else {
                                continue;
                            };
                            let key = format!("{fuel:?}");
                            match groups.iter_mut().find(|g| g.0 == key) {
                                Some(group) => group.1.push(ItemStack::new(item, 1)),
                                None => groups.push((key, vec![ItemStack::new(item, 1)])),
                            }
                        }
                        let fuels = groups.into_iter().flat_map(|g| g.1).collect();
                        out.push(GhostSlot {
                            target: GhostTarget::Fuel,
                            items: fuels,
                        });
                    }
                }
            }
        }
        // `GhostSlots.setSlot` keeps only slots with something to show.
        out.retain(|g| !g.items.is_empty());
        out
    }
    /// The recipe-book search tree (`SessionSearchTrees.updateRecipes`, a
    /// `FullTextSearchTree`): without a colon the text must occur in a
    /// tooltip line of a result item of the collection; `namespace:path`
    /// needs the namespace in a result id and the path in an id or a line.
    pub fn collection_matches(
        &self,
        entries: &[usize],
        query: &str,
        tooltip: &dyn Fn(&ItemStack) -> Vec<String>,
    ) -> bool {
        let query = query.to_lowercase();
        let results: Vec<&ItemStack> = entries
            .iter()
            .flat_map(|&i| self.entries[i].results())
            .collect();
        let in_lines = |text: &str| {
            results
                .iter()
                .any(|r| tooltip(r).iter().any(|l| l.to_lowercase().contains(text)))
        };
        let id = |r: &&ItemStack| {
            let (ns, p) = r.item.split_once(':').unwrap_or(("minecraft", &r.item));
            (ns.to_lowercase(), p.to_lowercase())
        };
        let Some((namespace, path)) = query.split_once(':') else {
            // `FullTextSearchTree.searchPlainText`: the plain-text tree only,
            // not the ids (#15321).
            return in_lines(&query);
        };
        let (namespace, path) = (namespace.trim(), path.trim());
        results.iter().any(|r| id(r).0.contains(namespace))
            && (results.iter().any(|r| id(r).1.contains(path)) || in_lines(path))
    }
}

/// Where a ghost recipe's stacks are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhostTarget {
    Result,
    /// A crafting grid slot, row by row.
    Grid(usize),
    /// A furnace's input and fuel slots.
    Input,
    Fuel,
}
/// One slot of a ghost recipe and the stacks it cycles through.
#[derive(Clone, Debug)]
pub struct GhostSlot {
    pub target: GhostTarget,
    pub items: Vec<ItemStack>,
}

/// A `RecipeDisplayId` as a `HashMap` key: a record of one int hashes as it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DisplayKey(i32);
impl horizon_core::java_hash_map::JavaKey for DisplayKey {
    fn java_hash(&self) -> i32 {
        self.0
    }
    fn tie_break(&self, other: &Self) -> std::cmp::Ordering {
        if self.0 < other.0 {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    }
}

/// `ClientRecipeBook`: the displays the client was sent, in a `HashMap` whose
/// iteration order (the display ids' buckets, then arrival order) decides the
/// order of the book's buttons.
#[derive(Default)]
pub struct ClientBook {
    known: horizon_core::java_hash_map::JavaHashMap<DisplayKey, ()>,
    pub highlight: std::collections::HashSet<i32>,
}
impl ClientBook {
    pub fn add(&mut self, display: i32) {
        self.known.put(DisplayKey(display), ());
    }
    pub fn remove(&mut self, display: i32) {
        self.known.remove(&DisplayKey(display));
        self.highlight.remove(&display);
    }
    /// `clear` (a replacing `ClientboundRecipeBookAddPacket`): the table keeps its size.
    pub fn clear(&mut self) {
        self.known.clear();
        self.highlight.clear();
    }
    /// The known display ids in iteration order.
    pub fn displays(&self) -> Vec<i32> {
        self.known.entries().map(|(k, _)| k.0).collect()
    }
    /// `rebuildCollections` then `getCollection(tab)`: per category, a
    /// collection per entry without a group or per group, in the order the
    /// known displays iterate; the search tab lists its categories in turn.
    pub fn collections(&self, book: &Book, book_type: BookType, tab: Tab) -> Vec<Collection> {
        let by_display: HashMap<i32, usize> = book
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.display_id, i))
            .collect();
        let mut by_category: HashMap<Category, Vec<Collection>> = HashMap::new();
        let mut groups: HashMap<(Category, String), (Category, usize)> = HashMap::new();
        for display in self.displays() {
            let Some(&i) = by_display.get(&display) else {
                continue;
            };
            let e = &book.entries[i];
            let list = by_category.entry(e.category).or_default();
            match &e.group {
                None => list.push(Collection { entries: vec![i] }),
                Some(g) => match groups.get(&(e.category, g.clone())) {
                    Some(&(_, at)) => list[at].entries.push(i),
                    None => {
                        groups.insert((e.category, g.clone()), (e.category, list.len()));
                        list.push(Collection { entries: vec![i] });
                    }
                },
            }
        }
        let categories: Vec<Category> = match tab {
            Tab::Search => book_type.search_categories().to_vec(),
            Tab::Category(c) => vec![c],
        };
        categories
            .into_iter()
            .flat_map(|c| by_category.remove(&c).unwrap_or_default())
            .collect()
    }
}

/// How a server's packet names things: an item by its registry id, a recipe
/// book category by its id, and the items of a tag.
pub struct PacketNames<'a> {
    pub item: &'a dyn Fn(i32) -> Option<String>,
    pub category: &'a dyn Fn(i32) -> Option<String>,
    pub tag: &'a dyn Fn(&str) -> Vec<String>,
}
impl PacketNames<'_> {
    fn holder_set(&self, set: &HolderSet) -> Vec<String> {
        match set {
            HolderSet::Tag(tag) => (self.tag)(tag.strip_prefix('#').unwrap_or(tag)),
            HolderSet::Items(ids) => ids.iter().filter_map(|&id| (self.item)(id)).collect(),
        }
    }
    /// The stacks a slot display shows, in order (`SlotDisplay.resolveForStacks`):
    /// what a display builds from a component or a dye is not reproduced, it
    /// shows its source.
    fn stacks(&self, display: &SlotDisplay, out: &mut Vec<ItemStack>) {
        match display {
            SlotDisplay::Empty | SlotDisplay::AnyFuel => {}
            SlotDisplay::Item(id) => out.extend((self.item)(*id).map(|n| ItemStack::new(&n, 1))),
            SlotDisplay::ItemStack(stack) => out.extend(
                (self.item)(stack.item).map(|n| ItemStack::new(&n, stack.count.max(1) as u32)),
            ),
            SlotDisplay::Tag(set) => {
                out.extend(self.holder_set(set).iter().map(|n| ItemStack::new(n, 1)))
            }
            SlotDisplay::WithAnyPotion(source) | SlotDisplay::OnlyWithComponent(source, _) => {
                self.stacks(source, out)
            }
            SlotDisplay::Dyed { target, .. } => self.stacks(target, out),
            SlotDisplay::SmithingTrim { base, .. } => self.stacks(base, out),
            SlotDisplay::WithRemainder { input, .. } => self.stacks(input, out),
            SlotDisplay::Composite(list) => list.iter().for_each(|d| self.stacks(d, out)),
        }
    }
    /// A slot display as an ingredient: the items it shows, without repeats.
    fn ingredient(&self, display: &SlotDisplay) -> Ingredient {
        let mut stacks = Vec::new();
        self.stacks(display, &mut stacks);
        let mut items: Vec<String> = Vec::new();
        for stack in stacks {
            if !items.iter().any(|i| **i == *stack.item) {
                items.push(stack.item.to_string());
            }
        }
        Ingredient::of(items)
    }
}

/// What a server's entry comes to in this book.
pub enum FromPacket {
    /// The book's entry (its id is `server:<display id>`), with the ingredients
    /// its craftable filter checks.
    Shown(Box<Entry>, Option<Vec<Ingredient>>),
    /// This book has no place for it: a stonecutter or smithing display, a
    /// category without a tab, no result.
    Unshowable,
    /// Its category id is not one this client knows (a registry it lacks): not
    /// a judgement of the entry.
    Unknown,
}

/// An entry of a server's `ClientboundRecipeBookAddPacket` as this book's entry.
pub fn entry_from_packet(e: &RecipeBookEntry, names: &PacketNames) -> FromPacket {
    let Some(category_name) = (names.category)(e.category) else {
        return FromPacket::Unknown;
    };
    shown(e, names, &category_name).unwrap_or(FromPacket::Unshowable)
}
fn shown(e: &RecipeBookEntry, names: &PacketNames, category_name: &str) -> Option<FromPacket> {
    let category = Category::from_name(category_name)?;
    let (display, result) = match &e.display {
        RecipeDisplay::Shapeless {
            ingredients,
            result,
            ..
        } => (
            Display::Shapeless(ingredients.iter().map(|d| names.ingredient(d)).collect()),
            result,
        ),
        RecipeDisplay::Shaped {
            width,
            height,
            ingredients,
            result,
            ..
        } => (
            Display::Shaped {
                width: usize::try_from(*width).ok()?,
                height: usize::try_from(*height).ok()?,
                cells: ingredients
                    .iter()
                    .map(|d| (!matches!(d, SlotDisplay::Empty)).then(|| names.ingredient(d)))
                    .collect(),
            },
            result,
        ),
        RecipeDisplay::Furnace {
            ingredient, result, ..
        } => (Display::Furnace(names.ingredient(ingredient)), result),
        RecipeDisplay::Stonecutter { .. } | RecipeDisplay::Smithing { .. } => return None,
    };
    let mut results = Vec::new();
    names.stacks(result, &mut results);
    let first = results.first()?.clone();
    let variants = if results.len() > 1 {
        results
    } else {
        Vec::new()
    };
    let requirements = e.requirements.as_ref().map(|sets| {
        sets.iter()
            .map(|set| Ingredient::of(names.holder_set(set)))
            .collect()
    });
    let entry = Entry {
        id: format!("server:{}", e.id),
        display_id: e.id,
        category,
        group: (e.group > 0).then(|| format!("server:{}", e.group - 1)),
        display,
        result: first,
        variants,
        stacks: Vec::new(),
    };
    Some(FromPacket::Shown(Box::new(entry), requirements))
}

/// The items an ingredient accepts, sorted.
fn items_of(i: &Ingredient) -> Vec<&str> {
    let mut items: Vec<&str> = i.items.iter().map(String::as_str).collect();
    items.sort_unstable();
    items
}
/// The items each ingredient of an entry accepts, in any order of ingredients.
fn sorted_sets(e: &Entry) -> Vec<Vec<&str>> {
    let mut sets: Vec<Vec<&str>> = e.requirements().into_iter().map(items_of).collect();
    sets.sort_unstable();
    sets
}
/// Whether the book's own entry is the recipe a server's entry describes: the
/// same result, category and display. A shaped recipe must have the same size
/// and each cell, empty ones included, accepting the same items; a shapeless or
/// smelting one, the same ingredients in any order.
fn same_recipe(own: &Entry, server: &Entry) -> bool {
    if own.category != server.category || own.result.item != server.result.item {
        return false;
    }
    match (&own.display, &server.display) {
        (
            Display::Shaped {
                width: w,
                height: h,
                cells,
            },
            Display::Shaped {
                width: sw,
                height: sh,
                cells: scells,
            },
        ) => {
            (w, h) == (sw, sh)
                && cells.len() == scells.len()
                && cells
                    .iter()
                    .zip(scells)
                    .all(|(a, b)| a.as_ref().map(items_of) == b.as_ref().map(items_of))
        }
        (Display::Shapeless(_), Display::Shapeless(_))
        | (Display::Furnace(_), Display::Furnace(_)) => sorted_sets(own) == sorted_sets(server),
        _ => false,
    }
}

impl Book {
    /// The display ids the server sent for `recipe_id` that this book numbers
    /// the same way, in the book's order. Empty for a recipe the server sent
    /// no entry for, for one it numbers differently from this book (the
    /// number may name another recipe here: a name is not tied to it), and for
    /// an unknown name.
    pub fn confirmed_ids(&self, recipe_id: &str) -> std::sync::Arc<[i32]> {
        let by_recipe = self.confirmed_by_recipe.get_or_init(|| {
            let mut ids = HashMap::<String, Vec<i32>>::new();
            for e in &self.entries {
                if self.confirmed.contains(&e.display_id) {
                    ids.entry(e.id.clone()).or_default().push(e.display_id);
                }
            }
            ids.into_iter().map(|(id, ids)| (id, ids.into())).collect()
        });
        static NONE: std::sync::LazyLock<std::sync::Arc<[i32]>> =
            std::sync::LazyLock::new(|| std::sync::Arc::from([]));
        by_recipe.get(recipe_id).unwrap_or(&NONE).clone()
    }
    /// The server removed `display`: it no longer vouches for that number.
    pub fn unconfirm(&mut self, display: i32) {
        if self.confirmed.remove(&display) {
            self.confirmed_by_recipe = Default::default();
        }
    }
    /// Puts a server's entries over this book. The book numbers its own
    /// recipes the way a server of the same recipes does, but a server with
    /// a plugin's recipes, a datapack or another order numbers them
    /// otherwise: the client of vanilla keeps the entries the packet carries
    /// (`ClientRecipeBook.add`), so each is taken as it came. The book's own
    /// entry stands when it is the same recipe (`same_recipe`), so a server of
    /// this client's own recipes shows what it always did, and the entries
    /// taken join the groups of the ones kept. Where the server's entry cannot
    /// be shown here, the book's own entry of that number goes: it is another
    /// recipe. An entry of a category this client does not know changes nothing.
    pub fn use_server_entries<'a>(
        &mut self,
        entries: impl IntoIterator<Item = &'a RecipeBookEntry>,
        names: &PacketNames,
    ) {
        enum Act {
            Keep,
            Take(Box<Entry>, Option<Vec<Ingredient>>),
            Hide,
        }
        let mut acts = Vec::new();
        self.confirmed_by_recipe = Default::default();
        for e in entries {
            // Each entry is judged afresh: a number stays confirmed only if this one matches.
            self.confirmed.remove(&e.id);
            let at = self.entry_of_display(e.id);
            let act = match entry_from_packet(e, names) {
                FromPacket::Unknown => Act::Keep,
                FromPacket::Unshowable => Act::Hide,
                FromPacket::Shown(entry, requirements) => match at {
                    Some(i) if same_recipe(&self.entries[i], &entry) => {
                        // The kept entry names its server group: the entries
                        // already taken from it take that name too.
                        if e.group > 0
                            && let Some(name) = self.entries[i].group.clone()
                        {
                            let old = self.server_groups.insert(e.group, name.clone());
                            if let Some(old) =
                                old.filter(|old| *old != name && old.starts_with("server:"))
                            {
                                for taken in &mut self.entries {
                                    if taken.id.starts_with("server:")
                                        && taken.group.as_deref() == Some(old.as_str())
                                    {
                                        taken.group = Some(name.clone());
                                    }
                                }
                            }
                        }
                        // A synthetic `server:` entry is the server's own and names no recipe.
                        if !self.entries[i].id.starts_with("server:") {
                            self.server_requirements.remove(&e.id);
                            if e.id >= 0 {
                                self.confirmed.insert(e.id);
                            }
                        }
                        Act::Keep
                    }
                    _ => Act::Take(entry, requirements),
                },
            };
            acts.push((e, at, act));
        }
        for (e, at, act) in acts {
            match act {
                Act::Keep => {}
                Act::Hide => {
                    if let Some(i) = at {
                        self.entries[i].display_id = -1;
                    }
                    self.server_requirements.remove(&e.id);
                }
                Act::Take(mut entry, requirements) => {
                    if e.group > 0 {
                        entry.group = Some(
                            self.server_groups
                                .entry(e.group)
                                .or_insert_with(|| format!("server:{}", e.group - 1))
                                .clone(),
                        );
                    }
                    match at {
                        Some(i) => self.entries[i] = *entry,
                        None => self.entries.push(*entry),
                    }
                    self.server_requirements.insert(e.id, requirements);
                }
            }
        }
    }
}

/// What the server knows of each recipe of the datapack: whether it is
/// special (`Recipe.isSpecial`: never learnable) and how many recipe book
/// displays it has (`RecipeManager.listDisplaysForRecipe`).
#[derive(Clone, Debug, Default)]
pub struct RecipeIndex {
    recipes: std::collections::BTreeMap<String, Known>,
}
#[derive(Clone, Debug)]
struct Known {
    special: bool,
    displays: usize,
    first_display: i32,
    /// `RecipeDisplay.craftingStation`: the workstation item of its displays.
    station: &'static str,
    /// `Recipe.showNotification`: unlocking it shows the `RecipeToast`.
    notify: bool,
    /// `result.id`, for the recipes the book doesn't list.
    result: Option<String>,
}
/// `RecipeDisplay.craftingStation` of a recipe type.
pub(crate) fn crafting_station(kind: &str) -> &'static str {
    match kind {
        "minecraft:smelting" => "minecraft:furnace",
        "minecraft:blasting" => "minecraft:blast_furnace",
        "minecraft:smoking" => "minecraft:smoker",
        "minecraft:campfire_cooking" => "minecraft:campfire",
        "minecraft:stonecutting" => "minecraft:stonecutter",
        "minecraft:brewing" => "minecraft:brewing_stand",
        "minecraft:smithing_transform" | "minecraft:smithing_trim" => "minecraft:smithing_table",
        _ => "minecraft:crafting_table",
    }
}
impl RecipeIndex {
    /// Index the merged and conditioned recipe resources.
    pub fn load(root: &std::path::Path, overlay: &PackOverlay) -> Self {
        match horizon_items::recipe_sources::sources(root, overlay) {
            Ok(sources) => Self::load_from_sources(&sources),
            Err(e) => {
                log::warn!("Horizon: recipes not shown in the book: {e}");
                Self::default()
            }
        }
    }
    /// The index of the recipe files `sources` (`Recipes::sources`' shape:
    /// a vanilla recipe by bare name, another as `<ns>:<name>`), numbered as
    /// a server holding exactly these recipes numbers its displays.
    pub fn load_from_sources(sources: &[(String, serde_json::Value)]) -> Self {
        Self::from_source_refs(
            sources
                .iter()
                .map(|(id, value)| (crate::crafting::qualified(id), value)),
        )
    }

    /// Build book displays only from retained recipes with placement candidates.
    pub(crate) fn from_recipes(recipes: &Recipes) -> Self {
        Self::from_source_refs(recipes.by_type.values().flat_map(|entries| {
            entries
                .iter()
                .filter(|entry| entry.has_placement_candidates())
                .map(|entry| (entry.id.clone(), &*entry.json))
        }))
    }

    pub(crate) fn from_serialized(recipes: &std::collections::BTreeMap<String, Box<str>>) -> Self {
        let values: Vec<_> = recipes
            .iter()
            .filter_map(|(id, text)| serde_json::from_str(text).ok().map(|value| (id, value)))
            .collect();
        Self::from_source_refs(values.iter().map(|(id, value)| ((*id).clone(), value)))
    }

    fn from_source_refs<'a>(
        sources: impl IntoIterator<Item = (String, &'a serde_json::Value)>,
    ) -> Self {
        let mut recipes = std::collections::BTreeMap::new();
        let mut insert = |id: String, v: &serde_json::Value| {
            let kind = v["type"].as_str().unwrap_or_default();
            let special = mod_api::__private::parts_installation::crafting_recipe(kind).is_some()
                || kind.starts_with("minecraft:crafting_special_")
                || kind == "minecraft:crafting_decorated_pot"
                // A brewing recipe has no recipe book display of its own
                // (`BrewingRecipe`), so it never takes display numbers.
                || kind == "minecraft:brewing";
            let displays = if special {
                0
            } else if kind == "minecraft:crafting_transmute" {
                // TransmuteRecipe.display: one per material count.
                match &v["material_count"] {
                    serde_json::Value::Object(o) => {
                        let min = o.get("min").and_then(|m| m.as_u64()).unwrap_or(1);
                        let max = o.get("max").and_then(|m| m.as_u64()).unwrap_or(8);
                        (max.saturating_sub(min) + 1) as usize
                    }
                    _ => 1,
                }
            } else {
                1
            };
            recipes.insert(
                id,
                Known {
                    special,
                    displays,
                    first_display: 0,
                    station: crafting_station(kind),
                    notify: v["show_notification"].as_bool().unwrap_or(true),
                    result: horizon_world::loot::recipe_result_id(&v["result"]),
                },
            );
        };
        for (id, value) in sources {
            insert(id, value);
        }
        // RecipeManager.unpackRecipeInfo numbers the displays in registration
        // order (path, then namespace), not in the order of the ids' text.
        let mut order: Vec<_> = recipes
            .iter_mut()
            .map(|(id, r)| (crate::crafting::registration_key(id), r))
            .collect();
        order.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut next = 0;
        for (_, r) in order {
            r.first_display = next;
            next += r.displays as i32;
        }
        Self { recipes }
    }
    pub fn contains(&self, id: &str) -> bool {
        self.recipes.contains_key(id)
    }
    pub fn special(&self, id: &str) -> bool {
        self.recipes.get(id).is_some_and(|r| r.special)
    }
    pub fn displays(&self, id: &str) -> usize {
        self.recipes.get(id).map_or(0, |r| r.displays)
    }
    /// The crafting station shown on its displays' toast, when unlocking it
    /// shows one (`Recipe.showNotification`).
    pub fn toast_station(&self, id: &str) -> Option<&'static str> {
        self.recipes.get(id).filter(|r| r.notify).map(|r| r.station)
    }
    /// The result item id its file names (none for trims).
    pub fn result(&self, id: &str) -> Option<&str> {
        self.recipes.get(id).and_then(|r| r.result.as_deref())
    }
    /// Every recipe id, in id order.
    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.recipes.keys()
    }
    /// The display ids of a recipe (none for a special one).
    pub fn display_ids(&self, id: &str) -> std::ops::Range<i32> {
        self.recipes.get(id).map_or(0..0, |r| {
            r.first_display..r.first_display + r.displays as i32
        })
    }
    /// The recipe a display id belongs to (`RecipeManager.getRecipeFromDisplay`).
    pub fn recipe_of_display(&self, display: i32) -> Option<&str> {
        self.recipes
            .iter()
            .find(|(_, r)| {
                (r.first_display..r.first_display + r.displays as i32).contains(&display)
            })
            .map(|(id, _)| id.as_str())
    }
    pub fn first_display(&self, id: &str) -> Option<i32> {
        self.recipes
            .get(id)
            .filter(|r| r.displays > 0)
            .map(|r| r.first_display)
    }
}

/// The recipe ids a knowledge book names: its `minecraft:recipes` component
/// (`Recipe.KEY_CODEC.listOf()`, the namespace defaulting to minecraft).
fn knowledge_recipes(stack: &ItemStack) -> Vec<String> {
    let Some(Tag::List(ids)) = stack.data().and_then(|d| d.other("minecraft:recipes")) else {
        return Vec::new();
    };
    ids.iter()
        .filter_map(|t| match t {
            Tag::String(id) => Some(id.to_string_lossy()),
            _ => None,
        })
        .map(|id| {
            if id.contains(':') {
                id
            } else {
                format!("minecraft:{id}")
            }
        })
        .collect()
}

/// `ServerRecipeBook`: the player's `recipeBook` compound, the known recipes
/// and the ones still to highlight (`toBeDisplayed`) with the open and
/// filter settings of each `RecipeBookType`.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// `isGuiOpen`-style flags per `BookType`.
    pub open: [bool; 4],
    pub filtering: [bool; 4],
    /// Known recipe ids (`recipes`).
    pub recipes: BTreeSet<String>,
    /// Recipes not yet shown in the book (`toBeDisplayed`): they animate once.
    pub to_be_displayed: BTreeSet<String>,
    /// The flags of mods' own books, beside the four vanilla pairs (arismoko/horizon#2415).
    pub custom: CustomBooks,
}
impl Settings {
    /// `ServerRecipeBook.addRecipes`: the displays added (the count
    /// `/recipe give` reports) and the recipes now known, in order.
    pub fn add_recipes(&mut self, index: &RecipeIndex, ids: &[String]) -> (usize, Vec<String>) {
        let mut displays = 0;
        let mut added = Vec::new();
        for id in ids {
            // The whole id: the crafting copy names a `minecraft:` recipe
            // bare, and every reader of the book (the network copy, the
            // advancements' JSON, the saved file) knows it qualified.
            let id = crate::crafting::qualified(id);
            if index.contains(&id) && !self.recipes.contains(&id) && !index.special(&id) {
                self.recipes.insert(id.clone());
                self.to_be_displayed.insert(id.clone());
                displays += index.displays(&id);
                added.push(id);
            }
        }
        (displays, added)
    }
    /// `KnowledgeBookItem.use` for the stack in the acting hand: `Ok` is its
    /// `InteractionResult.SUCCESS`, with the recipe ids for `ServerPlayer.awardRecipes`; `Err` is
    /// `FAIL`, with the recipe id vanilla logs (`Invalid recipe: {}`) or
    /// `None` for a book that names none. The book is used up first, as
    /// vanilla consumes it before either check (`ItemStack.consume(1, player)`;
    /// a player with infinite materials keeps it).
    pub fn knowledge_book_recipes(
        index: &RecipeIndex,
        hand: &mut Option<ItemStack>,
        infinite: bool,
    ) -> Result<Vec<String>, Option<String>> {
        let recipes = hand.as_ref().map_or_else(Vec::new, knowledge_recipes);
        if !infinite && let Some(stack) = hand.as_mut() {
            stack.count -= 1;
            if stack.count == 0 {
                *hand = None;
            }
        }
        if recipes.is_empty() {
            return Err(None);
        }
        // `RecipeManager.byKey`: one id this server has no recipe for stops
        // the use, the recipes beside it unlearned.
        if let Some(i) = recipes.iter().position(|id| !index.contains(id)) {
            return Err(Some(recipes[i].clone()));
        }
        Ok(recipes)
    }
    /// `ServerRecipeBook.removeRecipes`: the display ids removed, which its
    /// `ClientboundRecipeBookRemovePacket` lists (a transmute recipe has one
    /// per material count).
    pub fn remove_recipes(&mut self, index: &RecipeIndex, ids: &[String]) -> Vec<i32> {
        let mut displays = Vec::new();
        for id in ids {
            let id = crate::crafting::qualified(id);
            if self.recipes.remove(&id) {
                self.to_be_displayed.remove(&id);
                displays.extend(index.display_ids(&id));
            }
        }
        displays
    }
    /// `removeHighlight` (`ServerboundRecipeBookSeenRecipePacket`).
    pub fn remove_highlight(&mut self, id: &str) {
        self.to_be_displayed.remove(id);
    }
    pub fn knows(&self, id: &str) -> bool {
        self.recipes.contains(id)
    }
    /// The compound as saved, every recipe kept.
    pub fn read(tag: Option<&Tag>) -> Self {
        Self::read_valid(tag, &|_| true)
    }
    /// `ServerRecipeBook.loadUntrusted`: unknown recipes are dropped. Without
    /// both lists the compound does not decode and nothing is read.
    /// Mods' book flags are read on their own, so lists that do not decode keep them.
    pub fn read_valid(tag: Option<&Tag>, valid: &dyn Fn(&str) -> bool) -> Self {
        let mut s = Self {
            custom: CustomBooks::read(tag),
            ..Self::default()
        };
        let Some(tag) = tag else { return s };
        let ids = |key: &str| match tag.get(key) {
            Some(Tag::List(list)) => Some(
                list.iter()
                    .filter_map(|t| match t {
                        Tag::String(id) => Some(id.to_string_lossy()),
                        _ => None,
                    })
                    .map(|id| {
                        if id.contains(':') {
                            id
                        } else {
                            format!("minecraft:{id}")
                        }
                    })
                    .filter(|id| valid(id))
                    .collect::<BTreeSet<String>>(),
            ),
            _ => None,
        };
        let (Some(recipes), Some(to_be_displayed)) = (ids("recipes"), ids("toBeDisplayed")) else {
            return s;
        };
        let flag = |k: &str| matches!(tag.get(k), Some(Tag::Byte(b)) if *b != 0);
        for b in BookType::ALL {
            let (open, filter) = b.keys();
            s.open[b.index()] = flag(open);
            s.filtering[b.index()] = flag(filter);
        }
        s.recipes = recipes;
        s.to_be_displayed = to_be_displayed;
        s
    }
    /// The compound written to the player file (vanilla's lists follow an
    /// identity hash order; these are sorted).
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
        let list = |ids: &BTreeSet<String>| {
            Tag::List(ids.iter().map(|i| Tag::String(i.as_str().into())).collect())
        };
        entries.push(("recipes", list(&self.recipes)));
        entries.push(("toBeDisplayed", list(&self.to_be_displayed)));
        if let Some(custom) = self.custom.write() {
            entries.push((custom_books::EXTENSION, custom));
        }
        Tag::compound(entries)
    }
}

#[cfg(test)]
mod tests;
