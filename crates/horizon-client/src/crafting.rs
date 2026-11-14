//! Crafting and cooking recipes from the pinned data pack: shaped patterns
//! (`ShapedRecipePattern.matches`), shapeless recipes (a bipartite match of
//! ingredients to stacks, as `StackedContents.canCraft`), the special and
//! transmuting crafting recipes (`CustomRecipe` and friends), and
//! furnace-type cooking lookups.
use crate::{
    Result,
    item_data::StackData,
    items::{ItemStack, Items},
    particles::{DYE_FIREWORK_COLORS, FireworkExplosion, FireworkShape},
};
use horizon_core::nbt::{JavaString, Tag};
use horizon_world::native_load_context::LoopContext;
use horizon_world::{component_patch::Patch, loot::PackOverlay};
mod snapshots;
pub use snapshots::{
    FireworkShapeIngredient, RecipeRule, RecipeSnapshot, RecipeTemplate, RecipeTrimPattern,
};
mod cooking;
mod ingredient;
pub mod mod_kinds;
pub mod native_hooks;
#[cfg(test)]
mod registration_tests;
mod remainders;
mod repair_item;
#[path = "crafting/shapeless_matching.rs"]
mod shapeless_matching;
pub use ingredient::{Ingredient, Shape};
use serde_json::Value;
use shapeless_matching::matches_all;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

/// `PlacementInfo`: the ingredients to find and, per recipe slot, the index
/// of its ingredient (`slotsToIngredientIndex`, -1 for an empty slot). Empty
/// when the recipe is impossible to place.
#[derive(Clone, Debug, Default)]
pub struct Placement {
    pub ingredients: Vec<Ingredient>,
    pub slots: Vec<i32>,
    /// A `ShapedRecipe`'s width and height (the others fill the grid).
    pub shaped: Option<(usize, usize)>,
}
impl Placement {
    /// `PlacementInfo.createFromOptionals` (and `create` with no empty slots).
    pub fn of(cells: Vec<Option<Ingredient>>, shaped: Option<(usize, usize)>) -> Self {
        let mut out = Self {
            shaped,
            ..Self::default()
        };
        for cell in cells {
            match cell {
                Some(i) if i.items.is_empty() => return Self::default(),
                Some(i) => {
                    out.slots.push(out.ingredients.len() as i32);
                    out.ingredients.push(i);
                }
                None => out.slots.push(-1),
            }
        }
        out
    }
    pub fn impossible(&self) -> bool {
        self.slots.is_empty()
    }
}
/// A recipe result: `ItemStackTemplate` (`id`, `count`, `components`), or a bare item id
/// (`ItemStackTemplate.CODEC` falls back to `Item.CODEC`), with the id optional for the
/// transmuting recipes that keep the input's item.
#[derive(Clone, Debug)]
pub struct Template {
    pub item: Option<Arc<str>>,
    pub count: u32,
    pub damage: u32,
    pub extra: Option<Box<StackData>>,
    /// Result patch before decoding defaults, including explicit removals/zeroes.
    pub components: Option<Tag>,
}
/// `ItemStackTemplate.CODEC` count: absent means 1, present must be an
/// integer in `1..=99` (`ExtraCodecs.intRange(1, 99)`), else the recipe is
/// not loaded.
pub(crate) fn template_count(v: &Value) -> Option<u32> {
    match v.get("count") {
        None => Some(1),
        Some(c) => c
            .as_u64()
            .filter(|n| (1..=99).contains(n))
            .map(|n| n as u32),
    }
}
impl Template {
    pub(crate) fn parse(v: &Value) -> Option<Self> {
        let components = v.get("components").map(json_tag);
        let (damage, extra) = crate::item_data::decode_components(components.as_ref());
        Some(Self {
            item: horizon_world::loot::recipe_result_id(v).map(Arc::from),
            count: template_count(v)?,
            damage,
            extra,
            components,
        })
    }
    pub(crate) fn view(&self) -> crate::crafting::RecipeTemplate {
        crate::crafting::RecipeTemplate::new(
            self.item.as_deref().map(str::to_owned),
            self.count,
            self.components.clone(),
        )
    }
    /// `ItemStackTemplate.create`.
    pub fn create(&self) -> ItemStack {
        ItemStack {
            item: self
                .item
                .clone()
                .unwrap_or_else(|| Arc::from("minecraft:air")),
            count: self.count,
            damage: self.damage,
            extra: self.extra.clone(),
        }
    }
    /// `TransmuteRecipe.createWithOriginalComponents`: the input's component
    /// patch on the result item, then the template's own components over it.
    pub(crate) fn over(&self, input: &ItemStack, extra_count: u32) -> ItemStack {
        let original = assembly_components(input);
        let result = self
            .components
            .clone()
            .unwrap_or_else(|| assembly_components(&self.create()));
        // ItemStackTemplate.apply: the second patch wins for each component;
        // a removal replaces a value rather than coexisting with it.
        let entries = |tag: Tag| match tag {
            Tag::Compound(map) => map
                .into_iter()
                .map(|(key, value)| {
                    let key = key.to_string_lossy();
                    match key.strip_prefix('!') {
                        Some(key) => (key.to_owned(), None),
                        None => (key.to_owned(), Some(value)),
                    }
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let patch = Patch::new(entries(original).into_iter().chain(entries(result)));
        let components = Tag::compound(patch.entries().iter().map(|(key, value)| match value {
            Some(value) => (JavaString::from(key.as_str()), value.clone()),
            None => (
                JavaString::from(format!("!{key}").as_str()),
                Tag::Compound(Default::default()),
            ),
        }));
        let (damage, extra) = crate::item_data::decode_components(Some(&components));
        let mut out = ItemStack {
            item: self.item.clone().unwrap_or_else(|| input.item.clone()),
            count: self.count + extra_count,
            damage,
            extra,
        };
        if let Some(input) = input.data() {
            // Runtime-only stack bookkeeping is not a saved component patch.
            let data = out.data_mut();
            data.wire = input.wire.clone();
            data.bundle_selected = input.bundle_selected;
        }
        out.tidy();
        out
    }
}
/// Assembly also carries the transient map postprocess component; the saved
/// ItemStack codec deliberately omits it. Do not turn it into saved data.
fn assembly_components(stack: &ItemStack) -> Tag {
    let mut components = match crate::item_data::encode_components(stack) {
        Some(Tag::Compound(map)) => map,
        _ => Default::default(),
    };
    if let Some(postprocess) = stack.data().and_then(|d| d.map_post_processing) {
        components.insert(
            "minecraft:map_post_processing".into(),
            Tag::Int(i32::from(postprocess)),
        );
    }
    Tag::Compound(components)
}
/// A JSON recipe value as the NBT a component codec reads.
pub(crate) fn json_tag(v: &Value) -> Tag {
    match v {
        Value::Null => Tag::End,
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i32::try_from(i).map_or(Tag::Long(i), Tag::Int),
            None => Tag::Double(n.as_f64().unwrap_or(0.0).to_bits()),
        },
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(a) => Tag::List(a.iter().map(json_tag).collect()),
        Value::Object(o) => Tag::compound(o.iter().map(|(k, v)| (k.as_str(), json_tag(v)))),
    }
}
#[derive(Clone, Debug)]
pub enum Recipe {
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<Ingredient>>,
        result: Template,
    },
    Shapeless {
        ingredients: Vec<Ingredient>,
        result: Template,
    },
    /// The recipes with code of their own.
    Special(Special),
    /// A resolved owner and its opaque recipe value.
    Mod(mod_kinds::ModRecipe),
}
impl Recipe {
    /// `Recipe.isSpecial`: special recipes (e.g. firework rocket, dye) are not in the recipe book.
    pub fn is_special(&self) -> bool {
        matches!(self, Recipe::Special(_) | Recipe::Mod(_))
    }
}
/// The custom and transmuting crafting recipes.
#[derive(Clone, Debug)]
pub enum Special {
    /// A patterned banner and a blank one of the same colour copy the patterns.
    BannerDuplicate {
        banner: Ingredient,
        result: Template,
    },
    /// A banner decorates an undecorated shield.
    ShieldDecoration {
        banner: Ingredient,
        target: Ingredient,
        result: Template,
    },
    /// `TransmuteRecipe`: one input, 1..8 materials; shulker box and bundle
    /// dyeing, map cloning.
    Transmute {
        input: Ingredient,
        material: Ingredient,
        min: usize,
        max: usize,
        add_count: bool,
        result: Template,
    },
    /// `DyeRecipe`: dyed armour.
    Dye {
        target: Ingredient,
        dye: Ingredient,
        result: Template,
    },
    /// `ImbueRecipe`: tipped arrows.
    Imbue {
        source: Ingredient,
        material: Ingredient,
        result: Template,
    },
    /// `BookCloningRecipe`.
    BookCloning {
        source: Ingredient,
        material: Ingredient,
        min: i32,
        max: i32,
        result: Template,
    },
    /// `FireworkRocketRecipe`.
    FireworkRocket {
        shell: Ingredient,
        fuel: Ingredient,
        star: Ingredient,
        result: Template,
    },
    /// `FireworkStarRecipe`.
    FireworkStar {
        dye: Ingredient,
        fuel: Ingredient,
        trail: Ingredient,
        twinkle: Ingredient,
        shapes: Vec<(FireworkShape, Ingredient)>,
        result: Template,
    },
    /// `FireworkStarFadeRecipe`.
    FireworkStarFade {
        dye: Ingredient,
        target: Ingredient,
        result: Template,
    },
    /// `MapExtendingRecipe`: a map ringed by paper, when its scale is under 4.
    MapExtending {
        map: Ingredient,
        material: Ingredient,
        result: Template,
    },
    /// `DecoratedPotRecipe`: four sherds or bricks.
    DecoratedPot {
        back: Ingredient,
        left: Ingredient,
        right: Ingredient,
        front: Ingredient,
        result: Template,
    },
    /// `RepairItemRecipe`: two damaged items of one kind.
    RepairItem,
}
impl Special {
    /// A data projection of this parse; concrete outputs still use `craft`.
    fn rule(&self) -> crate::crafting::RecipeRule {
        use crate::crafting::RecipeRule as Rule;
        match self {
            Self::BannerDuplicate { banner, result } => Rule::BannerDuplicate {
                banner: banner.clone(),
                result: result.view(),
            },
            Self::ShieldDecoration {
                banner,
                target,
                result,
            } => Rule::ShieldDecoration {
                banner: banner.clone(),
                target: target.clone(),
                result: result.view(),
            },
            Self::Transmute {
                input,
                material,
                min,
                max,
                add_count,
                result,
            } => Rule::Transmute {
                input: input.clone(),
                material: material.clone(),
                min: *min,
                max: *max,
                add_count: *add_count,
                result: result.view(),
            },
            Self::Dye {
                target,
                dye,
                result,
            } => Rule::Dye {
                target: target.clone(),
                dye: dye.clone(),
                result: result.view(),
            },
            Self::Imbue {
                source,
                material,
                result,
            } => Rule::Imbue {
                source: source.clone(),
                material: material.clone(),
                result: result.view(),
            },
            Self::BookCloning {
                source,
                material,
                min,
                max,
                result,
            } => Rule::BookCloning {
                source: source.clone(),
                material: material.clone(),
                min: *min,
                max: *max,
                result: result.view(),
            },
            Self::FireworkRocket {
                shell,
                fuel,
                star,
                result,
            } => Rule::FireworkRocket {
                shell: shell.clone(),
                fuel: fuel.clone(),
                star: star.clone(),
                result: result.view(),
            },
            Self::FireworkStar {
                dye,
                fuel,
                trail,
                twinkle,
                shapes,
                result,
            } => Rule::FireworkStar {
                dye: dye.clone(),
                fuel: fuel.clone(),
                trail: trail.clone(),
                twinkle: twinkle.clone(),
                shapes: shapes
                    .iter()
                    .map(|(shape, ingredient)| {
                        crate::crafting::FireworkShapeIngredient::new(*shape, ingredient.clone())
                    })
                    .collect(),
                result: result.view(),
            },
            Self::FireworkStarFade {
                dye,
                target,
                result,
            } => Rule::FireworkStarFade {
                dye: dye.clone(),
                target: target.clone(),
                result: result.view(),
            },
            Self::MapExtending {
                map,
                material,
                result,
            } => Rule::MapExtending {
                map: map.clone(),
                material: material.clone(),
                result: result.view(),
            },
            Self::DecoratedPot {
                back,
                left,
                right,
                front,
                result,
            } => Rule::DecoratedPot {
                back: back.clone(),
                left: left.clone(),
                right: right.clone(),
                front: front.clone(),
                result: result.view(),
            },
            Self::RepairItem => Rule::RepairItem,
        }
    }
    /// The recipe-book displays of a learnable (non-custom) crafting recipe:
    /// its layout, its result as the button shows it, and every stack the
    /// result display cycles through when it is more than that one
    /// (`SlotDisplay.resolveForStacks`).
    pub fn book_displays(&self) -> Vec<BookDisplay> {
        use crate::recipe_book::Display;
        let first_of =
            |all: &Vec<ItemStack>, single: ItemStack| all.first().cloned().unwrap_or(single);
        let shown = |display, result, variants| BookDisplay {
            display,
            result,
            variants,
            stacks: Vec::new(),
        };
        match self {
            Self::Transmute {
                input,
                material,
                min,
                max,
                add_count,
                result,
            } => (*min..=*max)
                .map(|n| {
                    let mut ingredients = vec![input.clone()];
                    ingredients.extend(std::iter::repeat_n(material.clone(), n));
                    let mut made = result.create();
                    if *add_count {
                        made.count += n as u32;
                    }
                    // The result keeps the input's item: any item of the ingredient.
                    let all: Vec<ItemStack> = if result.item.is_none() {
                        input
                            .order
                            .iter()
                            .map(|item| {
                                let mut stack = made.clone();
                                stack.item = Arc::from(item.as_str());
                                stack
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    shown(Display::Shapeless(ingredients), first_of(&all, made), all)
                })
                .collect(),
            Self::Dye {
                target,
                dye,
                result,
            } => {
                // DyedSlotDemo: the targets cycle fastest, 16 stacks at most.
                let (targets, dyes) = (&target.order, &dye.order);
                let all: Vec<ItemStack> = (0..targets.len() * dyes.len())
                    .map(|i| {
                        let mut made = result.create();
                        made.item = Arc::from(targets[i % targets.len()].as_str());
                        let name = dyes[i / targets.len()].trim_start_matches("minecraft:");
                        let color = crate::block_entity_render::DYES
                            .iter()
                            .find(|(c, _)| name.strip_suffix("_dye") == Some(c))
                            .map_or(crate::block_entity_render::DYES[0].1, |d| d.1);
                        made.data_mut().dyed_color = Some(color as i32);
                        made
                    })
                    .take(16)
                    .collect();
                let first = result.create();
                vec![shown(
                    Display::Shapeless(vec![target.clone(), dye.clone()]),
                    first_of(&all, first),
                    all,
                )]
            }
            Self::Imbue {
                source,
                material,
                result,
            } => {
                let cells = (0..9)
                    .map(|i| Some(if i == 4 { source } else { material }.clone()))
                    .collect();
                // Any potion's tipped arrow.
                let potions = |item: ItemStack| -> Vec<ItemStack> {
                    crate::brewing::POTIONS
                        .iter()
                        .map(|p| item.clone().with_potion(&format!("minecraft:{}", p.id)))
                        .collect()
                };
                let all = potions(result.create());
                // The source ingredient is shown as a lingering potion of each kind.
                let lingering = source
                    .order
                    .iter()
                    .flat_map(|item| potions(ItemStack::new(item, 1)))
                    .collect();
                vec![BookDisplay {
                    stacks: vec![(4, lingering)],
                    ..shown(
                        Display::Shaped {
                            width: 3,
                            height: 3,
                            cells,
                        },
                        first_of(&all, result.create()),
                        all,
                    )
                }]
            }
            _ => Vec::new(),
        }
    }
}
/// One recipe-book display of a special recipe.
pub struct BookDisplay {
    pub display: crate::recipe_book::Display,
    /// The result as the button shows it (the first of `variants`).
    pub result: ItemStack,
    /// Every stack the result display cycles through; empty for one stack.
    pub variants: Vec<ItemStack>,
    /// Ingredients shown as other stacks than their items, by index in
    /// `Entry::requirements`.
    pub stacks: Vec<(usize, Vec<ItemStack>)>,
}
/// `CraftingInput`: the occupied bounding box of the grid.
pub struct Input<'a> {
    pub width: usize,
    pub height: usize,
    /// Position of the box in the grid.
    pub left: usize,
    pub top: usize,
    cells: Vec<Option<&'a ItemStack>>,
}
impl<'a> Input<'a> {
    /// `CraftingInput.ofPositioned`; None for an empty grid.
    pub fn of(grid: &'a [Option<ItemStack>], size: usize) -> Option<Self> {
        let occupied: Vec<(usize, usize)> = (0..size * size)
            .filter(|&i| grid.get(i).is_some_and(|c| c.is_some()))
            .map(|i| (i % size, i / size))
            .collect();
        let (x0, x1) = (
            occupied.iter().map(|p| p.0).min()?,
            occupied.iter().map(|p| p.0).max()?,
        );
        let (y0, y1) = (
            occupied.iter().map(|p| p.1).min()?,
            occupied.iter().map(|p| p.1).max()?,
        );
        let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
        let cells = (0..width * height)
            .map(|i| grid[(y0 + i / width) * size + x0 + i % width].as_ref())
            .collect();
        Some(Self {
            width,
            height,
            left: x0,
            top: y0,
            cells,
        })
    }
    pub fn get(&self, x: usize, y: usize) -> Option<&'a ItemStack> {
        self.cells.get(y * self.width + x).copied().flatten()
    }
    /// The non-empty stacks in slot order.
    pub fn stacks(&self) -> Vec<&'a ItemStack> {
        self.cells.iter().flatten().copied().collect()
    }
    pub fn count(&self) -> usize {
        self.cells.iter().flatten().count()
    }
}
/// What recipes need to know about an item.
#[derive(Clone, Debug)]
struct Facts {
    /// Index into the dye colours when the item has the `dye` component.
    dye: Option<usize>,
    info: crate::items::ItemInfo,
    max_stack: u32,
}
/// The tables recipes consult.
pub struct Context<'a> {
    recipes: &'a Recipes,
    context: &'a LoopContext,
    /// A filled map's scale, when its data is known.
    pub map_scale: &'a dyn Fn(i32) -> Option<u8>,
}
impl Context<'_> {
    fn facts(&self, stack: &ItemStack) -> Option<&Facts> {
        self.recipes.facts.get(&stack.item)
    }
    /// `itemStack.has(DataComponents.DYE)`.
    fn dye(&self, stack: &ItemStack) -> Option<usize> {
        self.facts(stack).and_then(|f| f.dye)
    }
    fn max_stack(&self, item: &str) -> u32 {
        self.recipes.facts.get(item).map_or(64, |f| f.max_stack)
    }
}
impl Special {
    /// `Recipe.matches`.
    fn matches(&self, ctx: &Context, input: &Input) -> bool {
        use crate::banner;
        let stacks = input.stacks();
        match self {
            Self::BannerDuplicate { banner: ing, .. } => {
                if stacks.len() != 2 {
                    return false;
                }
                let mut color = None;
                let (mut target, mut source) = (false, false);
                for s in &stacks {
                    let Some(this) =
                        banner::banner_color(&s.item).filter(|_| ing.test_in(ctx.context, s))
                    else {
                        return false;
                    };
                    match color {
                        None => color = Some(this),
                        Some(c) if c != this => return false,
                        _ => {}
                    }
                    let layers = banner::layers(s).len();
                    if layers > banner::MAX_LAYERS {
                        return false;
                    }
                    let seen = if layers > 0 { &mut source } else { &mut target };
                    if *seen {
                        return false;
                    }
                    *seen = true;
                }
                source && target
            }
            Self::ShieldDecoration {
                banner: ing,
                target: tgt,
                ..
            } => {
                if stacks.len() != 2 {
                    return false;
                }
                let (mut patterned, mut clear) = (false, false);
                for s in &stacks {
                    if ing.test_in(ctx.context, s) && banner::banner_color(&s.item).is_some() {
                        if patterned {
                            return false;
                        }
                        patterned = true;
                    } else {
                        if !tgt.test_in(ctx.context, s) || clear || !banner::layers(s).is_empty() {
                            return false;
                        }
                        clear = true;
                    }
                }
                patterned && clear
            }
            Self::Transmute {
                input: base,
                material,
                min,
                max,
                add_count,
                result,
            } => {
                let n = stacks.len();
                if n < min + 1 || n > max + 1 {
                    return false;
                }
                let mut found: Option<&ItemStack> = None;
                let mut materials = 0;
                for s in &stacks {
                    if base.test_in(ctx.context, s) {
                        if found.is_some() {
                            return false;
                        }
                        found = Some(s);
                    } else {
                        if !material.test_in(ctx.context, s) {
                            return false;
                        }
                        materials += 1;
                        if materials > *max {
                            return false;
                        }
                    }
                }
                let Some(found) = found else {
                    return false;
                };
                if materials < *min || materials > *max {
                    return false;
                }
                let count = if *add_count {
                    materials as u32 + result.count
                } else {
                    result.count
                };
                if count != 1 {
                    return true;
                }
                let made = result.over(found, 0);
                made.count <= ctx.max_stack(&made.item) && !made.same_item_same_components(found)
            }
            Self::Dye { target, dye, .. } => {
                if stacks.len() < 2 {
                    return false;
                }
                let (mut has_target, mut has_dyes) = (false, false);
                for s in &stacks {
                    if target.test_in(ctx.context, s) {
                        if has_target {
                            return false;
                        }
                        has_target = true;
                    } else {
                        if !dye.test_in(ctx.context, s) || ctx.dye(s).is_none() {
                            return false;
                        }
                        has_dyes = true;
                    }
                }
                has_dyes && has_target
            }
            Self::Imbue {
                source, material, ..
            } => {
                if input.width != 3 || input.height != 3 || stacks.len() != 9 {
                    return false;
                }
                (0..3).all(|y| {
                    (0..3).all(|x| {
                        let ing = if x == 1 && y == 1 { source } else { material };
                        input.get(x, y).is_some_and(|s| ing.test_in(ctx.context, s))
                    })
                })
            }
            Self::BookCloning {
                source,
                material,
                min,
                max,
                ..
            } => {
                if stacks.len() < 2 {
                    return false;
                }
                let (mut has_material, mut has_source) = (false, false);
                for s in &stacks {
                    if source.test_in(ctx.context, s) {
                        let Some(book) = s.data().and_then(|d| d.written.as_ref()) else {
                            return false;
                        };
                        let generation = i32::from(book.generation);
                        if generation < *min || generation > *max || has_source {
                            return false;
                        }
                        has_source = true;
                    } else {
                        if !material.test_in(ctx.context, s) {
                            return false;
                        }
                        has_material = true;
                    }
                }
                has_source && has_material
            }
            Self::FireworkRocket {
                shell, fuel, star, ..
            } => {
                if stacks.len() < 2 {
                    return false;
                }
                let mut has_shell = false;
                let mut fuels = 0;
                for s in &stacks {
                    if shell.test_in(ctx.context, s) {
                        if has_shell {
                            return false;
                        }
                        has_shell = true;
                    } else if fuel.test_in(ctx.context, s) {
                        fuels += 1;
                        if fuels > 3 {
                            return false;
                        }
                    } else if !star.test_in(ctx.context, s) {
                        return false;
                    }
                }
                has_shell && fuels >= 1
            }
            Self::FireworkStar {
                dye,
                fuel,
                trail,
                twinkle,
                shapes,
                ..
            } => {
                if stacks.len() < 2 {
                    return false;
                }
                let (mut has_fuel, mut has_dye, mut has_shape) = (false, false, false);
                let (mut has_trail, mut has_twinkle) = (false, false);
                for s in &stacks {
                    if twinkle.test_in(ctx.context, s) {
                        if has_twinkle {
                            return false;
                        }
                        has_twinkle = true;
                    } else if trail.test_in(ctx.context, s) {
                        if has_trail {
                            return false;
                        }
                        has_trail = true;
                    } else if fuel.test_in(ctx.context, s) {
                        if has_fuel {
                            return false;
                        }
                        has_fuel = true;
                    } else if dye.test_in(ctx.context, s) && ctx.dye(s).is_some() {
                        has_dye = true;
                    } else {
                        if !shapes.iter().any(|(_, ing)| ing.test_in(ctx.context, s)) || has_shape {
                            return false;
                        }
                        has_shape = true;
                    }
                }
                has_fuel && has_dye
            }
            Self::FireworkStarFade { dye, target, .. } => {
                if stacks.len() < 2 {
                    return false;
                }
                let (mut has_dye, mut has_target) = (false, false);
                for s in &stacks {
                    if dye.test_in(ctx.context, s) && ctx.dye(s).is_some() {
                        has_dye = true;
                    } else {
                        if !target.test_in(ctx.context, s) || has_target {
                            return false;
                        }
                        has_target = true;
                    }
                }
                has_target && has_dye
            }
            Self::MapExtending { map, material, .. } => {
                if input.width != 3 || input.height != 3 || stacks.len() != 9 {
                    return false;
                }
                let ring = (0..3).all(|y| {
                    (0..3).all(|x| {
                        let ing = if x == 1 && y == 1 { map } else { material };
                        input.get(x, y).is_some_and(|s| ing.test_in(ctx.context, s))
                    })
                });
                if !ring {
                    return false;
                }
                stacks
                    .iter()
                    .find_map(|s| s.data().and_then(|d| d.map_id))
                    .and_then(|id| (ctx.map_scale)(id))
                    .is_some_and(|scale| scale < 4)
            }
            Self::DecoratedPot {
                back,
                left,
                right,
                front,
                ..
            } => {
                input.width == 3
                    && input.height == 3
                    && stacks.len() == 4
                    && input
                        .get(1, 0)
                        .is_some_and(|s| back.test_in(ctx.context, s))
                    && input
                        .get(0, 1)
                        .is_some_and(|s| left.test_in(ctx.context, s))
                    && input
                        .get(2, 1)
                        .is_some_and(|s| right.test_in(ctx.context, s))
                    && input
                        .get(1, 2)
                        .is_some_and(|s| front.test_in(ctx.context, s))
            }
            Self::RepairItem => repair_item::pair(ctx, &stacks).is_some(),
        }
    }
    /// `Recipe.assemble`; None is the empty stack.
    fn assemble(&self, ctx: &Context, input: &Input) -> Option<ItemStack> {
        use crate::banner;
        let stacks = input.stacks();
        let made = match self {
            Self::BannerDuplicate { result, .. } => {
                let source = stacks
                    .iter()
                    .find(|s| (1..=banner::MAX_LAYERS).contains(&banner::layers(s).len()))?;
                // TransmuteRecipe.createWithOriginalComponents: the source's components.
                result.over(source, 0)
            }
            Self::ShieldDecoration {
                banner: ing,
                target,
                result,
            } => {
                let (mut banner_stack, mut shield) = (None, None);
                for s in &stacks {
                    if ing.test_in(ctx.context, s) && banner::banner_color(&s.item).is_some() {
                        banner_stack = Some(*s);
                    } else if target.test_in(ctx.context, s) {
                        shield = Some(*s);
                    }
                }
                let (banner_stack, shield) = (banner_stack?, shield?);
                let mut out = result.over(shield, 0);
                let data = out.data_mut();
                data.other.retain(|(key, _)| {
                    key != "!minecraft:base_color" && key != "!minecraft:banner_patterns"
                });
                data.banner_patterns = banner::layers(banner_stack).to_vec();
                data.base_color = banner::banner_color(&banner_stack.item).map(Arc::from);
                out
            }
            Self::Transmute {
                input: base,
                material,
                add_count,
                result,
                ..
            } => {
                if *add_count {
                    let mut materials = 0;
                    let mut found = None;
                    for s in &stacks {
                        if base.test_in(ctx.context, s) {
                            found = Some(*s);
                        } else if material.test_in(ctx.context, s) {
                            materials += 1;
                        }
                    }
                    result.over(found?, materials)
                } else {
                    let s = stacks.iter().find(|s| base.test_in(ctx.context, s))?;
                    result.over(s, 0)
                }
            }
            Self::Dye {
                target,
                dye,
                result,
            } => {
                let mut dyes = Vec::new();
                let mut target_stack: Option<&ItemStack> = None;
                for s in &stacks {
                    if target.test_in(ctx.context, s) {
                        if target_stack.is_some() {
                            return None;
                        }
                        target_stack = Some(s);
                    } else {
                        if !dye.test_in(ctx.context, s) {
                            return None;
                        }
                        // getOrDefault(DYE, WHITE)
                        dyes.push(ctx.dye(s).unwrap_or(0));
                    }
                }
                let target_stack = target_stack?;
                if dyes.is_empty() {
                    return None;
                }
                let current = target_stack.data().and_then(|d| d.dyed_color);
                let mut out = result.over(target_stack, 0);
                out.data_mut().dyed_color = Some(apply_dyes(current, &dyes));
                out
            }
            Self::Imbue { result, .. } => {
                let source = input.get(1, 1)?;
                let mut out = result.create();
                out.data_mut().potion = source.data().and_then(|d| d.potion.clone());
                out
            }
            Self::BookCloning {
                source,
                material,
                result,
                ..
            } => {
                let mut copies = 0u32;
                let mut book: Option<&ItemStack> = None;
                for s in &stacks {
                    if source.test_in(ctx.context, s)
                        && s.data().is_some_and(|d| d.written.is_some())
                    {
                        if book.is_some() {
                            return None;
                        }
                        book = Some(s);
                    } else {
                        if !material.test_in(ctx.context, s) {
                            return None;
                        }
                        copies += 1;
                    }
                }
                let book = book?;
                // count - 1 can be -1 when no material was seen; matches() ensures one.
                let mut out = result.over(book, copies.saturating_sub(1));
                if let Some(w) = out.data_mut().written.as_mut() {
                    w.generation += 1;
                }
                out
            }
            Self::FireworkRocket {
                fuel, star, result, ..
            } => {
                let mut explosions = Vec::new();
                let mut fuels = 0u8;
                for s in &stacks {
                    if fuel.test_in(ctx.context, s) {
                        fuels += 1;
                    } else if star.test_in(ctx.context, s)
                        && let Some(e) = s.data().and_then(|d| d.firework_explosion.clone())
                    {
                        explosions.push(e);
                    }
                }
                let mut out = result.create();
                // A rocket's prototype is one flight and no explosions.
                if fuels != 1 || !explosions.is_empty() {
                    out.data_mut().fireworks = Some((fuels, explosions));
                }
                out
            }
            Self::FireworkStar {
                dye,
                trail,
                twinkle,
                shapes,
                result,
                ..
            } => {
                let mut shape = FireworkShape::SmallBall;
                let (mut has_twinkle, mut has_trail) = (false, false);
                let mut colors = Vec::new();
                for s in &stacks {
                    if let Some((found, _)) =
                        shapes.iter().find(|(_, ing)| ing.test_in(ctx.context, s))
                    {
                        shape = *found;
                    } else if twinkle.test_in(ctx.context, s) {
                        has_twinkle = true;
                    } else if trail.test_in(ctx.context, s) {
                        has_trail = true;
                    } else if dye.test_in(ctx.context, s) {
                        colors.push(DYE_FIREWORK_COLORS[ctx.dye(s).unwrap_or(0)].1);
                    }
                }
                let mut out = result.create();
                out.data_mut().firework_explosion = Some(FireworkExplosion {
                    shape,
                    colors,
                    fade: Vec::new(),
                    trail: has_trail,
                    twinkle: has_twinkle,
                });
                out
            }
            Self::FireworkStarFade {
                dye,
                target,
                result,
            } => {
                let mut colors = Vec::new();
                let mut target_stack = None;
                for s in &stacks {
                    if dye.test_in(ctx.context, s) {
                        colors.push(DYE_FIREWORK_COLORS[ctx.dye(s).unwrap_or(0)].1);
                    } else if target.test_in(ctx.context, s) {
                        target_stack = Some(*s);
                    }
                }
                let target_stack = target_stack?;
                if colors.is_empty() {
                    return None;
                }
                let mut out = result.over(target_stack, 0);
                let explosion = out
                    .data_mut()
                    .firework_explosion
                    .get_or_insert_with(FireworkExplosion::default);
                explosion.fade = colors;
                out
            }
            Self::MapExtending { result, .. } => {
                let map = stacks
                    .iter()
                    .find(|s| s.data().is_some_and(|d| d.map_id.is_some()))?;
                let mut out = result.over(map, 0);
                out.data_mut().map_post_processing = Some(1);
                out
            }
            Self::DecoratedPot { result, .. } => {
                let side = |x, y| {
                    input.get(x, y).map(|s| ItemStack {
                        count: 1,
                        ..s.clone()
                    })
                };
                let mut out = result.create();
                let sides = [side(1, 0), side(0, 1), side(2, 1), side(1, 2)];
                if !crate::item_data::all_bricks(&sides) {
                    out.data_mut().pot_decorations = Some(Box::new(sides));
                }
                out
            }
            Self::RepairItem => repair_item::assemble(ctx, &stacks)?,
        };
        let mut made = made;
        made.tidy();
        // ItemStackTemplate.validate: oversized stacks become empty.
        (made.count > 0 && made.count <= ctx.max_stack(&made.item)).then_some(made)
    }
}
/// `DyedItemColor.applyDyes` over dye indexes.
pub fn apply_dyes(current: Option<i32>, dyes: &[usize]) -> i32 {
    let (mut r, mut g, mut b, mut intensity, mut n) = (0i32, 0i32, 0i32, 0i32, 0i32);
    let mut add = |rgb: u32| {
        let (red, green, blue) = (
            (rgb >> 16 & 255) as i32,
            (rgb >> 8 & 255) as i32,
            (rgb & 255) as i32,
        );
        intensity += red.max(green.max(blue));
        r += red;
        g += green;
        b += blue;
        n += 1;
    };
    if let Some(c) = current {
        add(c as u32);
    }
    for d in dyes {
        add(crate::block_entity_render::DYES[*d].1);
    }
    let (red, green, blue) = (r / n, g / n, b / n);
    let average = intensity as f32 / n as f32;
    let result = red.max(green.max(blue)) as f32;
    let scale = |c: i32| (c as f32 * average / result) as i32;
    scale(red) << 16 | scale(green) << 8 | scale(blue)
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
/// An item stack as a recipe's `result` names it (`ItemStackTemplate`); none
/// with no `id` or a count outside 1 to 99.
pub fn recipe_result(v: &Value) -> Option<ItemStack> {
    let template = Template::parse(v)?;
    template.item.is_some().then(|| template.create())
}
/// Recipe serializers registered by the engine for pinned vanilla and brewing data.
const VANILLA_RECIPE_SERIALIZERS: [&str; 22] = [
    "minecraft:blasting",
    "minecraft:brewing",
    "minecraft:campfire_cooking",
    "minecraft:crafting_decorated_pot",
    "minecraft:crafting_dye",
    "minecraft:crafting_imbue",
    "minecraft:crafting_shaped",
    "minecraft:crafting_shapeless",
    "minecraft:crafting_special_bannerduplicate",
    "minecraft:crafting_special_bookcloning",
    "minecraft:crafting_special_firework_rocket",
    "minecraft:crafting_special_firework_star",
    "minecraft:crafting_special_firework_star_fade",
    "minecraft:crafting_special_mapextending",
    "minecraft:crafting_special_repairitem",
    "minecraft:crafting_special_shielddecoration",
    "minecraft:crafting_transmute",
    "minecraft:smelting",
    "minecraft:smithing_transform",
    "minecraft:smithing_trim",
    "minecraft:smoking",
    "minecraft:stonecutting",
];

/// The sort key of vanilla's recipe registration order, which first-match
/// lookups (`RecipeManager.getRecipeFor`'s `findFirst`), the stonecutter's
/// list and the recipe book's display numbers all follow:
/// `ResourceManagerRegistryLoadTask` sorts the resource files by `Identifier`
/// (path, then namespace) and `RecipeMap` keeps that order. The file's path
/// ends in `.json`, so `prefix-early` sorts before `prefix`. `name` is a
/// table name (`oak_planks`) or a whole id.
pub fn registration_key(name: &str) -> (String, String) {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    (format!("{path}.json"), namespace.to_owned())
}
/// The type of a recipe the game doesn't craft itself: one outside `minecraft`.
fn custom_type(v: &Value) -> Option<&str> {
    v["type"]
        .as_str()
        .filter(|t| t.contains(':') && !t.starts_with("minecraft:"))
}
pub struct Recipes {
    /// Decoded read-only recipes, retained for their serializers when synchronizing.
    pub mod_read_only: Vec<(String, mod_kinds::ModRecipe)>,
    mod_crafting: bool,
    pub crafting: Vec<(String, Recipe)>,
    pub cooking: Vec<Cooking>,
    /// Every effective recipe's file, by recipe type (`minecraft:smelting`,
    /// `mymod:crusher`), in file order: data packs merged, load conditions held.
    pub by_type: HashMap<String, std::sync::Arc<[crate::crafting::RecipeSnapshot]>>,
    /// `category` and `group` of each crafting recipe, by file name.
    pub meta: HashMap<String, (String, String)>,
    facts: HashMap<Arc<str>, Facts>,
    /// The `#minecraft:curse` enchantments.
    curses: HashSet<String>,
    /// The first cooking recipe of each kind taking each item, built at the
    /// first lookup (furnaces look theirs up every tick).
    cook_index: std::sync::OnceLock<CookIndex>,
    /// What mods added to the recipe property sets this load (`property_set`).
    property_additions: PropertyAdditions,
}
type CookIndex = HashMap<String, crate::fast_hash::FxHashMap<String, Vec<usize>>>;
/// `MinMaxBounds.Ints` as (min, max) with the given defaults.
/// `TransmuteRecipe`'s `material_count` as the recipe parser reads it: a
/// number is exact, an object's missing ends are 1 and 8, both kept in 1..8.
pub(crate) fn transmute_material_bounds(material_count: Option<&Value>) -> (usize, usize) {
    let (min, max) = bounds(material_count, (1, 1), (1, 8));
    (min.clamp(1, 8) as usize, max.clamp(1, 8) as usize)
}
fn bounds(v: Option<&Value>, default: (i64, i64), open: (i64, i64)) -> (i64, i64) {
    match v {
        Some(Value::Number(n)) => {
            let n = n.as_i64().unwrap_or(default.0);
            (n, n)
        }
        Some(o @ Value::Object(_)) => (
            o["min"].as_i64().unwrap_or(open.0),
            o["max"].as_i64().unwrap_or(open.1),
        ),
        _ => default,
    }
}
/// `Util.isSymmetrical` is not needed: both orientations are tried.
/// `ShapedRecipePattern.shrink`: the pattern without empty border rows and columns.
fn shrink(pattern: &[Vec<char>]) -> Vec<Vec<char>> {
    let rows: Vec<&Vec<char>> = pattern
        .iter()
        .skip_while(|r| r.iter().all(|c| *c == ' '))
        .collect();
    let mut rows: Vec<Vec<char>> = rows.into_iter().cloned().collect();
    while rows.last().is_some_and(|r| r.iter().all(|c| *c == ' ')) {
        rows.pop();
    }
    let left = rows
        .iter()
        .filter_map(|r| r.iter().position(|c| *c != ' '))
        .min();
    let right = rows
        .iter()
        .filter_map(|r| r.iter().rposition(|c| *c != ' '))
        .max();
    match (left, right) {
        (Some(l), Some(r)) => rows.iter().map(|row| row[l..=r].to_vec()).collect(),
        _ => Vec::new(),
    }
}
pub use horizon_items::recipe_sources::qualified;

/// The keys of a minecraft-typed recipe that hold an ingredient: one entry
/// or a list, what `Recipes`, the station tables and the network copy read.
pub(crate) const INGREDIENT_KEYS: [&str; 23] = [
    "ingredient",
    "ingredients",
    "base",
    "addition",
    "template",
    "input",
    "reagent",
    "banner",
    "target",
    "material",
    "dye",
    "source",
    "shell",
    "fuel",
    "star",
    "trail",
    "twinkle",
    "map",
    "back",
    "left",
    "right",
    "front",
    "item",
];
/// The keys whose values are a map of ingredients: a shaped recipe's symbols,
/// a firework star's shapes.
pub(crate) const INGREDIENT_MAPS: [&str; 2] = ["key", "shapes"];

/// One recipe file after its load's ingredient pass.
pub struct ResolvedRecipe {
    /// The id as `recipe_sources::sources` names it.
    pub name: String,
    /// The file as written: what mods and clients see.
    pub file: Arc<Value>,
    /// The file with the `ingredient` hook's answers in place, for the tables
    /// to parse. The file itself when it names no mod kind.
    pub resolved: Arc<Value>,
}

/// The recipes one load reads, each mod ingredient resolved exactly once
/// (`Ingredient::resolve`): the tables and the network copy of a server load
/// all read this, so the `ingredient` hook hears each occurrence once per load.
#[derive(Default)]
pub struct ResolvedSources {
    pub recipes: Vec<ResolvedRecipe>,
    // Both tables use the items of this owning load. Parse station rules once,
    // whether the crafting projection or the station tables are built first.
    stations: std::sync::OnceLock<Vec<Option<crate::stations::ParsedStationRecipe>>>,
    /// What mods added to the recipe property sets, asked once per load.
    pub property_additions: PropertyAdditions,
}
/// The items mods added to the recipe property sets
/// (`RecipeManager.RECIPE_PROPERTY_SETS`) in one load, by set key without
/// its `minecraft:` prefix (`furnace_input`). The one answer every reader of
/// the sets takes: the cooking, station and brewing acceptance and the
/// network copy.
#[derive(Clone, Debug, Default)]
pub struct PropertyAdditions(HashMap<String, HashSet<String>>);
#[cfg(test)]
#[path = "crafting/property_sets_tests.rs"]
mod property_sets_tests;
impl PropertyAdditions {
    /// Asks the `recipe_property_sets` hook once. An unknown set key and an
    /// item that is air are logged and skipped.
    fn ask(context: &horizon_world::native_load_context::LoadContext) -> Self {
        let mut asked = Vec::new();
        native_hooks::native_recipe_property_sets(context, &mut asked);
        let mut sets = HashMap::<String, HashSet<String>>::new();
        for addition in asked {
            let key = addition
                .set
                .strip_prefix("minecraft:")
                .unwrap_or(&addition.set);
            if !key.contains(':')
                && !crate::lan::recipes::PROPERTY_SETS
                    .iter()
                    .any(|(set, ..)| *set == key)
            {
                log::warn!("recipe_property_sets: unknown set {}", addition.set);
                continue;
            }
            let set = sets.entry(key.to_owned()).or_default();
            for item in addition.items {
                if item == "minecraft:air" {
                    log::warn!("recipe_property_sets: air skipped in {key}");
                } else {
                    set.insert(item);
                }
            }
        }
        Self(sets)
    }
    /// The items mods added to the set `key` (`furnace_input`).
    pub fn get(&self, key: &str) -> Option<&HashSet<String>> {
        self.0.get(key)
    }
    /// Custom sets, with fully qualified keys. Vanilla additions are read by
    /// their owning recipe projections instead.
    pub fn custom(&self) -> impl Iterator<Item = (&str, &HashSet<String>)> {
        self.0
            .iter()
            .filter(|(key, _)| key.contains(':'))
            .map(|(key, items)| (key.as_str(), items))
    }
    /// Whether a mod added `item` to the set `key`.
    pub fn adds(&self, key: &str, item: &str) -> bool {
        self.get(key).is_some_and(|set| set.contains(item))
    }
}
impl ResolvedSources {
    pub(crate) fn station_recipes(
        &self,
        items: &Items,
    ) -> &[Option<crate::stations::ParsedStationRecipe>] {
        self.stations.get_or_init(|| {
            self.recipes
                .iter()
                .map(|recipe| {
                    crate::stations::ParsedStationRecipe::parse(
                        qualified(&recipe.name),
                        &recipe.resolved,
                        items,
                    )
                })
                .collect()
        })
    }
    /// The recipes under `root` and `overlay`, resolved for `context`'s load.
    /// The pass's context carries the same resources' fluid tags, for the
    /// ingredient hooks' fluid ingredients.
    pub fn load(
        root: &Path,
        overlay: &PackOverlay,
        context: &horizon_world::native_load_context::LoadContext,
    ) -> Result<Self> {
        let context = context.with_fluid_tags(horizon_items::items::fluid_tag_list(root, overlay)?);
        Self::resolve(
            horizon_items::recipe_sources::sources(root, overlay)?,
            &context,
        )
    }
    /// `load`, an unreadable recipe folder leaving no recipes.
    pub fn load_or_empty(
        root: &Path,
        overlay: &PackOverlay,
        context: &horizon_world::native_load_context::LoadContext,
    ) -> Self {
        Self::load(root, overlay, context).unwrap_or_else(|e| {
            log::warn!("recipes: could not load recipe sources: {e}");
            Self::default()
        })
    }
    /// `sources` (`recipe_sources::sources`' shape), the hook asked once for
    /// each mod ingredient in the recipes the loader parses (minecraft's own
    /// types; a mod's recipe types are its own to read).
    pub fn resolve(
        mut sources: Vec<(String, Value)>,
        context: &horizon_world::native_load_context::LoadContext,
    ) -> Result<Self> {
        // Every consumer keeps vanilla's resource-file registration order.
        sources.sort_by_cached_key(|(name, _)| registration_key(name));
        for (name, file) in &sources {
            if let Some(serializer) = file["type"].as_str() {
                mod_kinds::recipe_type(name, serializer)?;
            }
        }
        let listened = mod_api::__private::listened(mod_api::HookId::Ingredient);
        let recipes = sources
            .into_iter()
            .filter_map(|(name, file)| {
                let mut resolved = file.clone();
                let parsed_here = resolved["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("minecraft:"));
                // Answers a file carries are forgeries; an unlisted slot
                // would otherwise keep one.
                let mut changed = Ingredient::forget_answers(&mut resolved);
                if parsed_here {
                    for key in INGREDIENT_KEYS {
                        if let Some(slot) = resolved.get_mut(key) {
                            changed |= Ingredient::resolve(slot, context);
                        }
                    }
                    for key in INGREDIENT_MAPS {
                        for slot in resolved
                            .get_mut(key)
                            .and_then(Value::as_object_mut)
                            .into_iter()
                            .flat_map(|map| map.values_mut())
                        {
                            changed |= Ingredient::resolve(slot, context);
                        }
                    }
                }
                if parsed_here {
                    let kinds: Vec<_> = INGREDIENT_KEYS
                        .iter()
                        .filter_map(|key| resolved.get(*key))
                        .chain(
                            INGREDIENT_MAPS
                                .iter()
                                .filter_map(|key| resolved.get(*key)?.as_object())
                                .flat_map(|map| map.values()),
                        )
                        .flat_map(Ingredient::custom_kinds)
                        .collect();
                    if !listened && let Some((kind, _)) = kinds.first() {
                        // No mod answers the `ingredient` hook: the kind is
                        // unregistered, and the whole file fails.
                        log::error!(
                            "recipes: {name} refused: ingredient kind {kind} is not registered by any mod"
                        );
                        return None;
                    }
                    for (kind, _) in kinds.iter().filter(|(_, empty)| *empty) {
                        log::warn!(
                            "recipes: {name}: ingredient kind {kind} matched no items; the recipe cannot be placed"
                        );
                    }
                }
                let file = Arc::new(file);
                let resolved = if changed {
                    Arc::new(resolved)
                } else {
                    Arc::clone(&file)
                };
                Some(ResolvedRecipe {
                    name,
                    file,
                    resolved,
                })
            })
            .collect();
        Ok(Self {
            recipes,
            property_additions: PropertyAdditions::ask(context),
            ..Self::default()
        })
    }
}

impl Recipes {
    pub fn empty() -> Self {
        Self {
            mod_read_only: Vec::new(),
            mod_crafting: false,
            crafting: Vec::new(),
            cooking: Vec::new(),
            by_type: HashMap::new(),
            meta: HashMap::new(),
            facts: HashMap::new(),
            curses: HashSet::new(),
            cook_index: std::sync::OnceLock::new(),
            property_additions: PropertyAdditions::default(),
        }
    }

    /// `RecipeManager.getRecipe`: look up a crafting recipe by its id name.
    pub fn recipe(&self, name: &str) -> Option<&Recipe> {
        let bare = name.strip_prefix("minecraft:").unwrap_or(name);
        self.crafting
            .iter()
            .find(|(n, _)| n == name || n == bare)
            .map(|(_, r)| r)
    }

    pub fn load(
        root: &Path,
        items: &Items,
        overlay: &PackOverlay,
        context: &horizon_world::native_load_context::LoadContext,
    ) -> Result<Self> {
        let sources = ResolvedSources::load(root, overlay, context)?;
        Self::from_resolved(root, items, overlay, &sources)
    }
    /// The tables from condition-selected recipe sources (recipe_sources::sources' shape).
    /// The owning source load or server has already decided conditions; resolve ingredients here.
    pub fn from_sources(
        root: &Path,
        items: &Items,
        overlay: &PackOverlay,
        sources: Vec<(String, Value)>,
        context: &horizon_world::native_load_context::LoadContext,
    ) -> Result<Self> {
        Self::from_resolved(
            root,
            items,
            overlay,
            &ResolvedSources::resolve(sources, context)?,
        )
    }
    /// The tables from the condition-selected recipes of one owning load. Unknown serializers
    /// refuse the generation; malformed known recipes keep the skip-one-file policy.
    /// root and overlay give the enchantment tags the tables keep.
    pub fn from_resolved(
        root: &Path,
        items: &Items,
        overlay: &PackOverlay,
        sources: &ResolvedSources,
    ) -> Result<Self> {
        for recipe in &sources.recipes {
            if let Some(serializer) = recipe.file["type"].as_str() {
                mod_kinds::recipe_type(&recipe.name, serializer)?;
            }
        }
        let mut mod_read_only = Vec::new();
        let mut crafting = Vec::new();
        let mut cooking = Vec::new();
        let mut by_type: HashMap<String, Vec<crate::crafting::RecipeSnapshot>> = HashMap::new();
        let mut meta = HashMap::new();
        for (
            ResolvedRecipe {
                name,
                file: json,
                resolved,
            },
            station,
        ) in sources.recipes.iter().zip(sources.station_recipes(items))
        {
            let mut entry = None;
            if let Some(kind) = json["type"].as_str() {
                let id = if name.contains(':') {
                    name.clone()
                } else {
                    format!("minecraft:{name}")
                };
                entry = Some(crate::crafting::RecipeSnapshot::new(
                    id,
                    mod_kinds::recipe_type(name, kind)?,
                    json.clone(),
                ));
            }
            if custom_type(json).is_some() {
                if let Some(recipe) = mod_kinds::decode(name, json)
                    && let Some(entry) = entry
                {
                    if entry.kind == "minecraft:crafting" {
                        crafting.push((name.clone(), Recipe::Mod(recipe)));
                    } else {
                        mod_read_only.push((name.clone(), recipe));
                    }
                    by_type.entry(entry.kind.clone()).or_default().push(entry);
                }
                continue;
            }
            if !Ingredient::validate_recipe(resolved, items, name) {
                continue;
            }
            let v = &**resolved;
            let text = |k: &str| v[k].as_str().unwrap_or("").to_owned();
            meta.insert(name.clone(), (text("category"), text("group")));
            let ing = |k: &str| Ingredient::parse(&v[k], items);
            let result = || Template::parse(&v["result"]);
            let special = |recipe: Option<Special>| recipe.map(Recipe::Special);
            let cooked = cooking.len();
            let mut parsed_by_game = true;
            let parsed: Option<Recipe> = match v["type"].as_str().unwrap_or("") {
                "minecraft:crafting_shaped" => {
                    let pattern: Vec<Vec<char>> = v["pattern"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|r| r.as_str().map(|s| s.chars().collect()))
                        .collect();
                    let pattern = shrink(&pattern);
                    let height = pattern.len();
                    let width = pattern.first().map_or(0, Vec::len);
                    let mut cells = Vec::with_capacity(width * height);
                    let mut ok = height > 0;
                    for row in &pattern {
                        for c in row {
                            if *c == ' ' {
                                cells.push(None);
                            } else {
                                match Ingredient::parse(&v["key"][c.to_string()], items) {
                                    Some(i) => cells.push(Some(i)),
                                    None => ok = false,
                                }
                            }
                        }
                    }
                    match (ok, result()) {
                        (true, Some(result)) => Some(Recipe::Shaped {
                            width,
                            height,
                            cells,
                            result,
                        }),
                        _ => None,
                    }
                }
                "minecraft:crafting_shapeless" => {
                    let ingredients: Option<Vec<_>> = v["ingredients"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|i| Ingredient::parse(i, items))
                        .collect();
                    match (ingredients, result()) {
                        (Some(ingredients), Some(result)) => Some(Recipe::Shapeless {
                            ingredients,
                            result,
                        }),
                        _ => None,
                    }
                }
                "minecraft:crafting_special_bannerduplicate" => special(
                    ing("banner")
                        .zip(result())
                        .map(|(banner, result)| Special::BannerDuplicate { banner, result }),
                ),
                "minecraft:crafting_special_shielddecoration" => {
                    special(ing("banner").zip(ing("target")).zip(result()).map(
                        |((banner, target), result)| Special::ShieldDecoration {
                            banner,
                            target,
                            result,
                        },
                    ))
                }
                "minecraft:crafting_transmute" => special((|| {
                    let (min, max) = transmute_material_bounds(v.get("material_count"));
                    Some(Special::Transmute {
                        input: ing("input")?,
                        material: ing("material")?,
                        min,
                        max,
                        add_count: v["add_material_count_to_result"].as_bool().unwrap_or(false),
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_dye" => special((|| {
                    Some(Special::Dye {
                        target: ing("target")?,
                        dye: ing("dye")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_imbue" => special((|| {
                    Some(Special::Imbue {
                        source: ing("source")?,
                        material: ing("material")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_bookcloning" => special((|| {
                    let (min, max) = bounds(v.get("allowed_generations"), (0, 1), (0, 2));
                    Some(Special::BookCloning {
                        source: ing("source")?,
                        material: ing("material")?,
                        min: min as i32,
                        max: max as i32,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_firework_rocket" => special((|| {
                    Some(Special::FireworkRocket {
                        shell: ing("shell")?,
                        fuel: ing("fuel")?,
                        star: ing("star")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_firework_star" => special((|| {
                    let mut shapes = Vec::new();
                    for (shape, value) in v["shapes"].as_object()? {
                        shapes.push((
                            FireworkShape::from_name(shape),
                            Ingredient::parse(value, items)?,
                        ));
                    }
                    Some(Special::FireworkStar {
                        dye: ing("dye")?,
                        fuel: ing("fuel")?,
                        trail: ing("trail")?,
                        twinkle: ing("twinkle")?,
                        shapes,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_firework_star_fade" => special((|| {
                    Some(Special::FireworkStarFade {
                        dye: ing("dye")?,
                        target: ing("target")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_mapextending" => special((|| {
                    Some(Special::MapExtending {
                        map: ing("map")?,
                        material: ing("material")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_decorated_pot" => special((|| {
                    Some(Special::DecoratedPot {
                        back: ing("back")?,
                        left: ing("left")?,
                        right: ing("right")?,
                        front: ing("front")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_repairitem" => special(Some(Special::RepairItem)),
                kind @ ("minecraft:smelting"
                | "minecraft:blasting"
                | "minecraft:smoking"
                | "minecraft:campfire_cooking") => {
                    if let (Some(ingredient), Some(result)) = (ing("ingredient"), result()) {
                        cooking.push(Cooking {
                            id: name.clone(),
                            category: text("category"),
                            group: text("group"),
                            kind: kind.to_owned(),
                            ingredient,
                            result: result.create(),
                            experience: v["experience"].as_f64().unwrap_or(0.0) as f32,
                            time: v["cookingtime"].as_u64().unwrap_or(200) as u32,
                        });
                    }
                    None
                }
                "minecraft:stonecutting"
                | "minecraft:smithing_transform"
                | "minecraft:smithing_trim" => None,
                _ => {
                    parsed_by_game = false;
                    None
                }
            };
            // `recipes_of_type` lists what the game parsed (a recipe it drops
            // is not in force), typed from that one parse; a type it doesn't
            // parse keeps only its file.
            if let Some(mut entry) = entry {
                entry.group = text("group");
                match (&parsed, cooking.get(cooked)) {
                    (Some(Recipe::Shaped { cells, result, .. }), _) => {
                        entry.ingredients = cells.iter().flatten().cloned().collect();
                        entry.results = vec![result.create()];
                    }
                    (
                        Some(Recipe::Shapeless {
                            ingredients,
                            result,
                        }),
                        _,
                    ) => {
                        entry.ingredients = ingredients.clone();
                        entry.results = vec![result.create()];
                    }
                    (Some(Recipe::Special(special)), _) => {
                        entry.rule = Some(special.rule());
                    }
                    (_, Some(cook)) => {
                        entry.ingredients = vec![cook.ingredient.clone()];
                        entry.results = vec![cook.result.clone()];
                        entry.cooking_time = Some(cook.time);
                        entry.experience = Some(cook.experience);
                    }
                    _ => {
                        entry.rule = station
                            .as_ref()
                            .map(crate::stations::ParsedStationRecipe::rule)
                    }
                }
                if parsed.is_some()
                    || cooking.len() > cooked
                    || entry.rule.is_some()
                    || !parsed_by_game
                {
                    by_type.entry(entry.kind.clone()).or_default().push(entry);
                }
            }
            if let Some(recipe) = parsed {
                crafting.push((name.clone(), recipe));
            }
        }
        let facts = items
            .items
            .iter()
            .map(|(id, info)| {
                (
                    id.clone(),
                    Facts {
                        dye: info.dye.as_deref().and_then(|d| {
                            crate::block_entity_render::DYES
                                .iter()
                                .position(|(name, _)| *name == d)
                        }),
                        info: info.clone(),
                        max_stack: info.max_stack,
                    },
                )
            })
            .collect();
        let curses = crate::items::load_tags(root, "enchantment", overlay)?
            .remove("minecraft:curse")
            .unwrap_or_default();
        Ok(Self {
            mod_read_only,
            mod_crafting: !mod_api::__private::parts_installation::crafting_recipe_kinds()
                .is_empty(),
            crafting,
            cooking,
            by_type: by_type
                .into_iter()
                .map(|(kind, files)| (kind, files.into()))
                .collect(),
            meta,
            facts,
            curses,
            cook_index: Default::default(),
            property_additions: sources.property_additions.clone(),
        })
    }
    /// Match a `size`x`size` grid (row-major) against the crafting recipes.
    pub fn craft(
        &self,
        context: &LoopContext,
        grid: &[Option<ItemStack>],
        size: usize,
    ) -> Option<ItemStack> {
        self.find(context, grid, size).map(|(_, result)| result)
    }
    /// `craft` with the matched recipe's id (the file name without namespace).
    pub fn find(
        &self,
        context: &LoopContext,
        grid: &[Option<ItemStack>],
        size: usize,
    ) -> Option<(&str, ItemStack)> {
        self.find_with(context, grid, size, &|_| None)
    }
    /// `RecipeManager.getRecipeFor(CRAFTING, ...)` with a lookup for the scale
    /// of a filled map (map extending reads the map's saved data).
    pub fn find_with(
        &self,
        context: &LoopContext,
        grid: &[Option<ItemStack>],
        size: usize,
        map_scale: &dyn Fn(i32) -> Option<u8>,
    ) -> Option<(&str, ItemStack)> {
        self.find_with_dimensions(context, grid, size, size, size, map_scale)
    }
    /// `Recipe.matches` then `assemble` for one recipe: None when it does
    /// not match, Some(None) when it matches with nothing to make.
    fn assemble_if_matches(
        ctx: &Context,
        input: &Input,
        recipe: &Recipe,
    ) -> Option<Option<ItemStack>> {
        match recipe {
            Recipe::Mod(_) => None,
            Recipe::Special(special) => special
                .matches(ctx, input)
                .then(|| special.assemble(ctx, input)),
            Recipe::Shaped {
                width,
                height,
                cells,
                result,
            } => {
                if *width != input.width
                    || *height != input.height
                    || cells.iter().flatten().count() != input.count()
                {
                    return None;
                }
                let matches = |mirror: bool| {
                    (0..*height).all(|y| {
                        (0..*width).all(|x| {
                            let rx = if mirror { width - 1 - x } else { x };
                            match (&cells[y * width + rx], input.get(x, y)) {
                                (None, None) => true,
                                (Some(i), Some(s)) => i.test_in(ctx.context, s),
                                _ => false,
                            }
                        })
                    })
                };
                (matches(false) || matches(true)).then(|| Some(result.create()))
            }
            Recipe::Shapeless {
                ingredients,
                result,
            } => matches_all(ctx.context, ingredients, &input.stacks())
                .then(|| Some(result.create())),
        }
    }
    /// `recipe.matches(craftSlots.asCraftInput())` for one recipe by id
    /// (`minecraft:name`).
    pub fn recipe_matches(
        &self,
        context: &LoopContext,
        id: &str,
        grid: &[Option<ItemStack>],
        size: usize,
    ) -> bool {
        let name = id.trim_start_matches("minecraft:");
        let Some((_, recipe)) = self.crafting.iter().find(|(n, _)| n == name) else {
            return false;
        };
        if let Recipe::Mod(recipe) = recipe {
            return recipe.assemble(grid, size).is_some();
        }
        let Some(input) = Input::of(grid, size) else {
            return false;
        };
        let ctx = Context {
            recipes: self,
            context,
            map_scale: &|_| None,
        };
        Self::assemble_if_matches(&ctx, &input, recipe).is_some()
    }
    /// `Recipe.placementInfo` of a crafting or cooking recipe by id; None for
    /// an unknown id, an empty placement when it is impossible to place.
    pub fn placement(&self, id: &str) -> Option<Placement> {
        let name = id.trim_start_matches("minecraft:");
        if let Some(c) = self.cooking.iter().find(|c| c.id == name) {
            return Some(Placement::of(vec![Some(c.ingredient.clone())], None));
        }
        let (_, recipe) = self.crafting.iter().find(|(n, _)| n == name)?;
        Some(match recipe {
            Recipe::Shaped {
                width,
                height,
                cells,
                ..
            } => Placement::of(cells.clone(), Some((*width, *height))),
            Recipe::Shapeless { ingredients, .. } => {
                Placement::of(ingredients.iter().cloned().map(Some).collect(), None)
            }
            Recipe::Special(Special::Transmute {
                input,
                material,
                max,
                ..
            }) => {
                let mut list = vec![Some(input.clone())];
                list.extend(std::iter::repeat_n(Some(material.clone()), *max));
                Placement::of(list, None)
            }
            Recipe::Special(Special::Dye { target, dye, .. }) => {
                Placement::of(vec![Some(target.clone()), Some(dye.clone())], None)
            }
            Recipe::Special(Special::Imbue {
                source, material, ..
            }) => Placement::of(
                (0..9)
                    .map(|i| Some(if i == 4 { source } else { material }.clone()))
                    .collect(),
                None,
            ),
            // CustomRecipe.placementInfo: NOT_PLACEABLE.
            Recipe::Special(_) | Recipe::Mod(_) => Placement::default(),
        })
    }
    /// The `experience` of the cooking recipe with this file name, and 0 for one this data
    /// pack does not have (a save from another pack can name it).
    /// `id` with or without the `minecraft:` namespace (a furnace keeps the full id).
    pub fn cooking_experience(&self, id: &str) -> f32 {
        let id = id.strip_prefix("minecraft:").unwrap_or(id);
        self.cooking
            .iter()
            .find(|c| c.id == id)
            .map_or(0.0, |c| c.experience)
    }
    /// The mod-created sets of this load, with fully qualified keys.
    pub fn custom_property_sets(&self) -> impl Iterator<Item = (&str, &HashSet<String>)> {
        self.property_additions.custom()
    }
    /// What mods added to the property set `key` (`furnace_input`).
    pub fn property_set(&self, key: &str) -> Option<&HashSet<String>> {
        self.property_additions.get(key)
    }
    /// Whether a furnace of recipe type `kind` (`minecraft:smelting`) takes
    /// `input` (`RecipePropertySet.test`): a recipe of that type names it, or
    /// a mod added it to the kind's input set.
    pub fn cooking_accepts(&self, context: &LoopContext, kind: &str, input: &ItemStack) -> bool {
        self.cook_placeable(context, kind, input).is_some()
            || crate::lan::recipes::PROPERTY_SETS
                .iter()
                .any(|(set, recipe_type, _)| {
                    *recipe_type == kind && self.property_additions.adds(set, &input.item)
                })
    }
}

/// Resolves the remainder from the stack at the moment it is consumed.
pub trait CraftingRemainder {
    /// What remains after one item of this current stack is consumed.
    fn crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack>;
}
impl CraftingRemainder for crate::items::Items {
    fn crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        self.get(&stack.item)?
            .crafting_remainder
            .as_deref()
            .map(|id| ItemStack::new(id, 1))
    }
}

#[cfg(test)]
mod tests {
    fn serialized<'a>(
        recipes: &'a super::Recipes,
        serializer: &str,
    ) -> Vec<&'a super::RecipeSnapshot> {
        recipes
            .by_type
            .values()
            .flat_map(|entries| entries.iter())
            .filter(|recipe| recipe.json["type"] == serializer)
            .collect()
    }

    fn lc() -> &'static LoopContext {
        crate::test_resources::server_loop_context()
    }
    use super::*;
    use crate::banner::BannerLayer;
    use std::fs;

    /// ItemStackTemplate.apply applies the result patch after the input patch.
    #[test]
    fn transmute_result_patch_preserves_inheritance_and_explicit_removals() {
        let input_tag = json_tag(&serde_json::json!({
            "minecraft:damage": 5, "minecraft:lore": ["Old"]
        }));
        let (damage, extra) = crate::item_data::decode_components(Some(&input_tag));
        let input = ItemStack {
            item: "minecraft:wooden_sword".into(),
            count: 1,
            damage,
            extra,
        };
        for (components, expected_damage, removed) in [
            (
                serde_json::json!({"minecraft:custom_name": "Kept"}),
                5,
                false,
            ),
            (
                serde_json::json!({"minecraft:custom_name": "Up", "!minecraft:damage": {}}),
                0,
                true,
            ),
            (
                serde_json::json!({"minecraft:custom_name": "Zero", "minecraft:damage": 0}),
                0,
                false,
            ),
        ] {
            let template = Template::parse(&serde_json::json!({
                "id": "minecraft:stone_sword", "components": components
            }))
            .unwrap();
            let output = template.over(&input, 0);
            assert_eq!(output.item.as_ref(), "minecraft:stone_sword");
            assert_eq!(output.damage, expected_damage);
            let encoded = crate::item_data::encode_components(&output).unwrap();
            assert_eq!(
                encoded.get("minecraft:lore"),
                input_tag.get("minecraft:lore")
            );
            assert_eq!(
                encoded.get("minecraft:custom_name"),
                Some(&json_tag(&components["minecraft:custom_name"]))
            );
            assert_eq!(encoded.get("!minecraft:damage").is_some(), removed);
            // A result patch setting zero must survive until the output's prototype is
            // known: that prototype may itself start damaged after native registration.
            assert_eq!(
                encoded.get("minecraft:damage"),
                (!removed).then_some(&Tag::Int(expected_damage as i32))
            );
        }
        let mut map = input.clone();
        map.data_mut().map_post_processing = Some(1);
        let template = Template::parse(&serde_json::json!({"id": "minecraft:filled_map"})).unwrap();
        let output = template.over(&map, 0);
        assert_eq!(output.data().unwrap().map_post_processing, Some(1));
        assert!(
            crate::item_data::encode_components(&output)
                .unwrap()
                .get("minecraft:map_post_processing")
                .is_none()
        );
    }

    #[test]
    fn result_count_is_bounded_like_vanilla() {
        let parse = |s: &str| Template::parse(&serde_json::from_str(s).unwrap());
        assert_eq!(parse(r#"{"id":"minecraft:stone"}"#).unwrap().count, 1);
        let top = parse(r#"{"id":"minecraft:stone","count":99}"#);
        assert_eq!(top.unwrap().count, 99);
        for bad in [
            "0",
            "100",
            "2147483648",
            "4294967295",
            "4294967296",
            "-1",
            "1.5",
            "\"2\"",
        ] {
            let json = format!(r#"{{"id":"minecraft:stone","count":{bad}}}"#);
            assert!(parse(&json).is_none(), "count {bad} must be rejected");
        }
    }

    /// A mod's recipe whose ingredient is one of NeoForge's standard shapes
    /// loads and carries the resolved items, where the plain string parser
    /// used to drop the file whole.
    #[test]
    fn a_neoforge_ingredient_recipe_loads() {
        let root =
            std::env::temp_dir().join(format!("horizon-neo-ingredient-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        fs::create_dir_all(root.join("data/mymod/recipe")).unwrap();
        fs::write(
            root.join(horizon_items::content_namespaces::MARKER),
            br#"["mymod"]"#,
        )
        .unwrap();
        let recipe = r##"{
            "type": "minecraft:crafting_shapeless",
            "ingredients": [
                {"neoforge:ingredient_type": "neoforge:compound",
                 "children": ["minecraft:oak_planks", "minecraft:birch_planks"]},
                {"type": "neoforge:difference",
                 "base": ["minecraft:oak_planks", "minecraft:birch_planks"],
                 "subtracted": "minecraft:birch_planks"}
            ],
            "result": {"id": "minecraft:stick", "count": 4}
        }"##;
        fs::write(root.join("data/mymod/recipe/treated.json"), recipe).unwrap();
        let items = Items::for_tests(HashMap::new());
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert_eq!(recipes.crafting[0].0, "mymod:treated");
        let _ = fs::remove_dir_all(&root);
    }

    /// `recipes_of_type` lists what the game parsed, typed from that parse:
    /// a shaped recipe with an unreadable key is in force nowhere, so it is
    /// not listed, and the readable one carries its cells and result.
    #[test]
    fn recipes_of_type_lists_only_what_the_game_parsed() {
        let recipe = |s: &str| {
            let file = Arc::new(serde_json::from_str::<Value>(s).unwrap());
            (file.clone(), file)
        };
        let named = |name: &str, s: &str| {
            let (file, resolved) = recipe(s);
            ResolvedRecipe {
                name: format!("mymod:{name}"),
                file,
                resolved,
            }
        };
        let sources = ResolvedSources {
            recipes: vec![
                named(
                    "good",
                    r#"{"type":"minecraft:crafting_shaped","group":"g","pattern":["AA"," A"],
                        "key":{"A":"minecraft:stone"},"result":{"id":"minecraft:stick","count":2}}"#,
                ),
                named(
                    "bad",
                    r#"{"type":"minecraft:crafting_shaped","pattern":["AB"],
                        "key":{"A":"minecraft:stone","B":{"type":"mymod:nope"}},
                        "result":{"id":"minecraft:stick"}}"#,
                ),
            ],
            ..ResolvedSources::default()
        };
        let items = Items::for_tests(HashMap::new());
        let recipes =
            Recipes::from_resolved(&std::env::temp_dir(), &items, &PackOverlay::NONE, &sources)
                .unwrap();
        let listed = &serialized(&recipes, "minecraft:crafting_shaped");
        assert_eq!(listed.len(), 1, "the dropped recipe is not listed");
        assert_eq!(listed[0].id, "mymod:good");
        assert_eq!(listed[0].group, "g");
        assert_eq!(listed[0].ingredients.len(), 3, "its three filled cells");
        assert_eq!(listed[0].results[0].item.as_ref(), "minecraft:stick");
        assert_eq!(listed[0].results[0].count, 2);
    }

    #[test]
    fn recipe_rule_views_share_station_and_crafting_parses() {
        mod_platform::testing::with_recipe_types(&["example:machine"], || {
            let named = |name: &str, json: &str| {
                let file = Arc::new(serde_json::from_str::<Value>(json).unwrap());
                ResolvedRecipe {
                    name: name.into(),
                    resolved: file.clone(),
                    file,
                }
            };
            let sources = ResolvedSources {
                recipes: vec![
                    named(
                        "cut",
                        r#"{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"id":"minecraft:stick","count":2}}"#,
                    ),
                    named(
                        "invalid",
                        r#"{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"id":"minecraft:stick","count":100}}"#,
                    ),
                    named(
                        "smith",
                        r#"{"type":"minecraft:smithing_transform","template":"minecraft:stick","base":"minecraft:stone","addition":"minecraft:iron_ingot","result":{"id":"minecraft:diamond"}}"#,
                    ),
                    named(
                        "transmute",
                        r#"{"type":"minecraft:crafting_transmute","input":"minecraft:stone","material":"minecraft:stick","material_count":{"min":2,"max":4},"add_material_count_to_result":true,"result":{"id":"minecraft:diamond","count":2,"components":{"minecraft:damage":0,"!minecraft:custom_name":{}}}}"#,
                    ),
                    named(
                        "trim",
                        r#"{"type":"minecraft:smithing_trim","template":"minecraft:stick","base":"minecraft:stone","addition":"minecraft:iron_ingot","pattern":"minecraft:coast"}"#,
                    ),
                    named("custom", r#"{"type":"example:machine","value":42}"#),
                ],
                ..ResolvedSources::default()
            };
            let items = Items::for_tests(HashMap::new());
            let root = std::env::temp_dir();
            let recipes =
                Recipes::from_resolved(&root, &items, &PackOverlay::NONE, &sources).unwrap();
            let stations = crate::stations::StationRecipes::from_sources(&root, &items, &sources);
            let cuts = &recipes.by_type["minecraft:stonecutting"];
            assert_eq!(cuts.len(), 1);
            let Some(crate::crafting::RecipeRule::Stonecutting { input, result }) = &cuts[0].rule
            else {
                panic!("missing stonecutting rule")
            };
            assert_eq!(input.order, ["minecraft:stone"]);
            assert_eq!(result.count, 2);
            assert_eq!(stations.stonecutting[0].result, *result);
            let smith = &serialized(&recipes, "minecraft:smithing_transform")[0];
            let Some(crate::crafting::RecipeRule::SmithingTransform { base, result, .. }) =
                &smith.rule
            else {
                panic!("missing smithing rule")
            };
            assert_eq!(base.order, ["minecraft:stone"]);
            assert_eq!(result.item.as_deref(), Some("minecraft:diamond"));
            assert!(
                smith.results.is_empty(),
                "smithing output inherits components"
            );
            let transmute = &serialized(&recipes, "minecraft:crafting_transmute")[0];
            let Some(crate::crafting::RecipeRule::Transmute {
                min,
                max,
                add_count,
                result,
                ..
            }) = &transmute.rule
            else {
                panic!("missing transmute rule")
            };
            assert_eq!((*min, *max, *add_count), (2, 4, true));
            assert_eq!(result.item.as_deref(), Some("minecraft:diamond"));
            let patch = result
                .components
                .as_ref()
                .expect("component patch retained");
            assert!(patch.get("minecraft:damage").is_some());
            assert!(patch.get("!minecraft:custom_name").is_some());
            let trim = &serialized(&recipes, "minecraft:smithing_trim")[0];
            let Some(crate::crafting::RecipeRule::SmithingTrim {
                pattern, addition, ..
            }) = &trim.rule
            else {
                panic!("missing smithing trim rule")
            };
            assert!(
                matches!(pattern, crate::crafting::RecipeTrimPattern::Reference(id) if id == "minecraft:coast")
            );
            assert_eq!(addition.order, ["minecraft:iron_ingot"]);
            assert!(trim.results.is_empty());
            assert!(
                transmute.results.is_empty(),
                "grid-dependent output is not fixed"
            );
            let grid = [
                Some(ItemStack::new("minecraft:stone", 1)),
                Some(ItemStack::new("minecraft:stick", 1)),
                Some(ItemStack::new("minecraft:stick", 1)),
                None,
            ];
            let output = recipes.craft(lc(), &grid, 2).expect("transmute matches");
            assert_eq!(output.item.as_ref(), "minecraft:diamond");
            assert_eq!(output.count, 4);
            let custom = &recipes.by_type["example:machine"][0];
            assert!(custom.rule.is_none());
            assert_eq!(custom.json["value"], 42);
        });
    }

    #[test]
    fn recipe_template_patch_survives_special_and_smithing_parse() {
        let items = Items::for_tests(HashMap::new());
        let template = r#"{"id":"minecraft:diamond","count":2,"components":{"minecraft:custom_data":{"rule":1}}}"#;
        let named = |name: &str, json: String| {
            let file = Arc::new(serde_json::from_str::<Value>(&json).unwrap());
            ResolvedRecipe {
                name: name.into(),
                resolved: file.clone(),
                file,
            }
        };
        let sources = ResolvedSources {
            recipes: vec![
                named(
                    "banner",
                    format!(
                        r#"{{"type":"minecraft:crafting_special_bannerduplicate","banner":"minecraft:white_banner","result":{template}}}"#
                    ),
                ),
                named(
                    "shield",
                    format!(
                        r#"{{"type":"minecraft:crafting_special_shielddecoration","banner":"minecraft:white_banner","target":"minecraft:shield","result":{template}}}"#
                    ),
                ),
                named(
                    "smith",
                    format!(
                        r#"{{"type":"minecraft:smithing_transform","base":"minecraft:stone","result":{template}}}"#
                    ),
                ),
            ],
            ..ResolvedSources::default()
        };
        let recipes =
            Recipes::from_resolved(&std::env::temp_dir(), &items, &PackOverlay::NONE, &sources)
                .unwrap();
        for kind in [
            "minecraft:crafting_special_bannerduplicate",
            "minecraft:crafting_special_shielddecoration",
            "minecraft:smithing_transform",
        ] {
            let result = match serialized(&recipes, kind)[0].rule.as_ref().unwrap() {
                crate::crafting::RecipeRule::BannerDuplicate { result, .. }
                | crate::crafting::RecipeRule::ShieldDecoration { result, .. }
                | crate::crafting::RecipeRule::SmithingTransform { result, .. } => result,
                _ => panic!("wrong rule"),
            };
            assert_eq!(result.item.as_deref(), Some("minecraft:diamond"));
            assert_eq!(result.count, 2);
            assert_eq!(
                result
                    .components
                    .as_ref()
                    .unwrap()
                    .get("minecraft:custom_data")
                    .unwrap()
                    .get("rule"),
                Some(&Tag::Int(1))
            );
        }
    }

    #[test]
    fn all_special_rule_variants_project_from_loaded_recipes() {
        let cases = [
            (
                "crafting_special_bannerduplicate",
                r#"{"banner":"test:banner","result":"test:banner"}"#,
            ),
            (
                "crafting_special_shielddecoration",
                r#"{"banner":"test:banner","target":"test:shield","result":"test:shield"}"#,
            ),
            (
                "crafting_dye",
                r#"{"target":"test:target","dye":"test:dye","result":"test:result"}"#,
            ),
            (
                "crafting_imbue",
                r#"{"source":"test:source","material":"test:material","result":"test:result"}"#,
            ),
            (
                "crafting_special_bookcloning",
                r#"{"source":"test:source","material":"test:material","allowed_generations":{"min":0,"max":2},"result":"test:result"}"#,
            ),
            (
                "crafting_special_firework_rocket",
                r#"{"shell":"test:shell","fuel":"test:fuel","star":"test:star","result":"test:result"}"#,
            ),
            (
                "crafting_special_firework_star",
                r#"{"dye":"test:dye","fuel":"test:fuel","trail":"test:trail","twinkle":"test:twinkle","shapes":{"star":"test:shape"},"result":"test:result"}"#,
            ),
            (
                "crafting_special_firework_star_fade",
                r#"{"dye":"test:dye","target":"test:target","result":"test:result"}"#,
            ),
            (
                "crafting_special_mapextending",
                r#"{"map":"test:map","material":"test:material","result":"test:result"}"#,
            ),
            (
                "crafting_decorated_pot",
                r#"{"back":"test:back","left":"test:left","right":"test:right","front":"test:front","result":"test:result"}"#,
            ),
            ("crafting_special_repairitem", "{}"),
        ];
        let sources = ResolvedSources {
            recipes: cases
                .iter()
                .map(|(kind, json)| {
                    let mut v: Value = serde_json::from_str(json).unwrap();
                    v["type"] = format!("minecraft:{kind}").into();
                    let file = Arc::new(v);
                    ResolvedRecipe {
                        name: format!("test:{kind}"),
                        resolved: file.clone(),
                        file,
                    }
                })
                .collect(),
            ..ResolvedSources::default()
        };
        let items = Items::for_tests(HashMap::new());
        let recipes =
            Recipes::from_resolved(&std::env::temp_dir(), &items, &PackOverlay::NONE, &sources)
                .unwrap();
        for (kind, _) in cases {
            let entries = serialized(&recipes, &format!("minecraft:{kind}"));
            assert_eq!(entries.len(), 1, "{kind}");
            let rule = entries[0].rule.as_ref().expect(kind);
            use crate::crafting::RecipeRule as R;
            assert!(
                matches!(
                    (kind, rule),
                    (
                        "crafting_special_bannerduplicate",
                        R::BannerDuplicate { .. }
                    ) | (
                        "crafting_special_shielddecoration",
                        R::ShieldDecoration { .. }
                    ) | ("crafting_dye", R::Dye { .. })
                        | ("crafting_imbue", R::Imbue { .. })
                        | ("crafting_special_bookcloning", R::BookCloning { .. })
                        | ("crafting_special_firework_rocket", R::FireworkRocket { .. })
                        | ("crafting_special_firework_star", R::FireworkStar { .. })
                        | (
                            "crafting_special_firework_star_fade",
                            R::FireworkStarFade { .. }
                        )
                        | ("crafting_special_mapextending", R::MapExtending { .. })
                        | ("crafting_decorated_pot", R::DecoratedPot { .. })
                        | ("crafting_special_repairitem", R::RepairItem)
                ),
                "{kind}: {rule:?}"
            );
            assert!(entries[0].results.is_empty(), "{kind} is dynamic");
        }
    }

    /// NeoForge's standard ingredient shapes resolve to their flat item
    /// lists; a kind the loader doesn't know still fails the file.
    #[test]
    fn an_unclaimed_ingredient_kind_fails_and_dedup_keeps_order() {
        let items = Items::for_tests(HashMap::new());
        let json = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert!(
            Ingredient::parse(&json(r#"{"type":"mymod:aspect","aspect":"fire"}"#), &items)
                .is_none()
        );
        assert!(Ingredient::parse(&json(r#"{"type":"minecraft:future_shape"}"#), &items).is_none());
        assert!(Ingredient::parse(&json(r#"{"type":"aspect"}"#), &items).is_none());
        let mut list = vec!["b".to_string(), "a".to_string(), "b".to_string()];
        mod_api::__private::dedup_keep_order(&mut list);
        assert_eq!(list, vec!["b".to_string(), "a".to_string()]);
    }

    /// NeoForge's shapes keep their children, and matching a concrete
    /// stack asks each child its own way — any for compound, all for
    /// intersection, base-but-not-subtracted for difference. A mod's own
    /// kind among the children answers through its hook the same way
    /// (proven on a staged build).
    #[test]
    fn nested_shapes_test_each_child_their_own_way() {
        let items = Items::for_tests(HashMap::new());
        let parse = |s: &str| {
            Ingredient::parse(&serde_json::from_str::<Value>(s).unwrap(), &items).unwrap()
        };
        let stack = |n: &str| ItemStack {
            item: n.into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let compound = parse(
            r#"{"type":"neoforge:compound","children":["minecraft:oak_planks","minecraft:birch_planks"]}"#,
        );
        assert!(compound.children.is_some(), "the children stay with it");
        assert!(compound.test_in(lc(), &stack("minecraft:oak_planks")));
        assert!(compound.test_in(lc(), &stack("minecraft:birch_planks")));
        assert!(!compound.test_in(lc(), &stack("minecraft:stone")));
        let inter = parse(
            r#"{"type":"neoforge:intersection","ingredients":["minecraft:oak_planks",["minecraft:oak_planks","minecraft:birch_planks"]]}"#,
        );
        assert!(inter.test_in(lc(), &stack("minecraft:oak_planks")));
        assert!(!inter.test_in(lc(), &stack("minecraft:birch_planks")));
        let diff = parse(
            r#"{"type":"neoforge:difference","base":["minecraft:oak_planks","minecraft:birch_planks"],"subtracted":"minecraft:birch_planks"}"#,
        );
        assert!(diff.test_in(lc(), &stack("minecraft:oak_planks")));
        assert!(!diff.test_in(lc(), &stack("minecraft:birch_planks")));
    }

    #[test]
    fn neoforge_ingredient_shapes_parse() {
        let mut tags = HashMap::new();
        tags.insert(
            "minecraft:planks".to_owned(),
            ["minecraft:oak_planks", "minecraft:birch_planks"]
                .iter()
                .map(|s| s.to_string())
                .collect::<HashSet<_>>(),
        );
        let items = Items::for_tests(tags);
        let json = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        // Compound (`CompoundIngredient`): any child matching is enough —
        // the union of their items, a tag beside an item gives both.
        let union = Ingredient::parse(
            &json(r##"{"neoforge:ingredient_type":"neoforge:compound","children":["#minecraft:planks","minecraft:stone"]}"##),
            &items)
        .unwrap();
        // The tag's items come from a HashSet: their order is not fixed.
        let mut all = union.order.clone();
        all.sort();
        assert_eq!(
            all,
            [
                "minecraft:birch_planks",
                "minecraft:oak_planks",
                "minecraft:stone"
            ]
        );
        // Intersection: every child must match — only the items all name.
        let both = Ingredient::parse(
            &json(r##"{"type":"neoforge:intersection","children":["#minecraft:planks","minecraft:oak_planks"]}"##),
            &items)
        .unwrap();
        assert_eq!(both.order, ["minecraft:oak_planks"]);
        // Difference: the base without the subtracted.
        let no_oak = Ingredient::parse(
            &json(r##"{"type":"neoforge:difference","base":"#minecraft:planks","subtracted":"minecraft:oak_planks"}"##),
            &items)
        .unwrap();
        assert_eq!(no_oak.order, ["minecraft:birch_planks"]);
        // An empty intersection and an unknown kind fail the file, as today.
        let empty = Ingredient::parse(
            &json(
                r#"{"type":"neoforge:intersection","children":["minecraft:oak_planks","minecraft:birch_planks"]}"#,
            ),
            &items,
        );
        assert!(empty.is_none());
        let unknown =
            Ingredient::parse(&json(r#"{"type":"mymod:aspect","aspect":"fire"}"#), &items);
        assert!(unknown.is_none());
    }

    fn ingredient(items: &[&str]) -> Ingredient {
        Ingredient::of(items.iter().map(|s| (*s).to_owned()).collect())
    }
    fn patterned(item: &str) -> ItemStack {
        let mut s = ItemStack::new(item, 1);
        s.data_mut().banner_patterns = vec![BannerLayer::new("minecraft:cross", "red")];
        s
    }
    /// About vanilla's count of cooking recipes: four kinds, ingredients of
    /// one to twelve items, overlapping (several recipes take some items).
    fn cooking_book() -> Recipes {
        let kinds = ["smelting", "blasting", "smoking", "campfire_cooking"];
        let cooking = (0..320usize)
            .map(|i| Cooking {
                id: format!("r{i}"),
                category: "misc".into(),
                group: String::new(),
                kind: kinds[i % 4].into(),
                ingredient: ingredient(
                    &(0..1 + i % 12)
                        .map(|k| format!("minecraft:item_{}", (i * 7 + k * 13) % 400))
                        .collect::<Vec<_>>()
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                ),
                result: ItemStack::new("minecraft:stone", 1),
                experience: 0.1,
                time: 200,
            })
            .collect();
        Recipes {
            mod_read_only: Vec::new(),
            mod_crafting: false,
            crafting: Vec::new(),
            cooking,
            by_type: HashMap::new(),
            meta: HashMap::new(),
            facts: HashMap::new(),
            curses: HashSet::new(),
            cook_index: Default::default(),
            property_additions: PropertyAdditions::default(),
        }
    }
    #[test]
    fn only_types_outside_minecraft_are_custom() {
        let kind = |json: &str| {
            let v: Value = serde_json::from_str(json).unwrap();
            custom_type(&v).map(str::to_owned)
        };
        assert_eq!(
            kind(r#"{"type":"mymod:crusher"}"#).as_deref(),
            Some("mymod:crusher")
        );
        assert_eq!(kind(r#"{"type":"minecraft:smelting"}"#), None);
        assert_eq!(kind(r#"{"type":"minecraft:from_a_later_version"}"#), None);
        assert_eq!(kind(r#"{"type":"no_namespace"}"#), None);
        assert_eq!(kind(r#"{"type":7}"#), None);
        assert_eq!(kind("{}"), None);
    }
    /// Recipes of every type land in `by_type`, in file order, with the id the
    /// file has; a data pack overrides by id and adds more; a recipe of a mod's
    /// own type is kept as its file says and isn't crafted.
    #[test]
    fn custom_recipes_load_by_type_with_pack_override() {
        mod_platform::testing::with_recipe_types(
            &["mymod:crusher", "mymod:mixer", "pack:press"],
            || {
                let root = std::env::temp_dir()
                    .join(format!("horizon-custom-recipes-{}", std::process::id()));
                let _ = fs::remove_dir_all(&root);
                fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
                fs::create_dir_all(root.join("data/mymod/recipe")).unwrap();
                fs::write(
                    root.join(horizon_items::content_namespaces::MARKER),
                    br#"["mymod"]"#,
                )
                .unwrap();
                let file = |kind: &str, tag: &str| {
                    format!(r#"{{"type":"{kind}","tag":"{tag}"}}"#).into_bytes()
                };
                fs::write(
                    root.join("data/mymod/recipe/a.json"),
                    file("mymod:crusher", "a"),
                )
                .unwrap();
                fs::write(
                    root.join("data/mymod/recipe/b.json"),
                    file("mymod:crusher", "b"),
                )
                .unwrap();
                fs::write(
                    root.join("data/mymod/recipe/c.json"),
                    file("mymod:mixer", "c"),
                )
                .unwrap();
                fs::create_dir_all(root.join("data/mymod/recipe/deep/er")).unwrap();
                fs::write(
                    root.join("data/mymod/recipe/deep/er/d.json"),
                    file("mymod:mixer", "d"),
                )
                .unwrap();
                fs::write(
                    root.join("data/minecraft/recipe/later.json"),
                    file("minecraft:from_a_later_version", "x"),
                )
                .unwrap();
                // A vanilla type needs ingredients the loader accepts to be a recipe at all.
                let cutting = |tag: &str| {
                    format!(
                r#"{{"type":"minecraft:stonecutting","tag":"{tag}","ingredient":"minecraft:stone","result":{{"id":"minecraft:stone_bricks"}}}}"#
            )
            .into_bytes()
                };
                fs::write(
                    root.join("data/minecraft/recipe/iron_ingot.json"),
                    cutting("vanilla-iron"),
                )
                .unwrap();
                let items = Items::for_tests(HashMap::new());
                let error = Recipes::load(
                    &root,
                    &items,
                    &PackOverlay::NONE,
                    &crate::test_resources::server_load(),
                )
                .err()
                .unwrap()
                .to_string();
                assert!(
                    error.contains("later.json")
                        && error.contains("minecraft:from_a_later_version"),
                    "{error}"
                );
                fs::remove_file(root.join("data/minecraft/recipe/later.json")).unwrap();
                let recipes = Recipes::load(
                    &root,
                    &items,
                    &PackOverlay::NONE,
                    &crate::test_resources::server_load(),
                )
                .unwrap();
                let ids = |kind: &str| -> Vec<String> {
                    recipes.by_type[kind].iter().map(|r| r.id.clone()).collect()
                };
                assert_eq!(ids("mymod:crusher"), ["mymod:a", "mymod:b"]);
                assert_eq!(ids("mymod:mixer"), ["mymod:c", "mymod:deep/er/d"]);
                // A `minecraft` type 26.3 doesn't have is no recipe, as in `RecipeManager`.
                assert!(
                    !recipes
                        .by_type
                        .contains_key("minecraft:from_a_later_version")
                );
                // Vanilla's types answer through the same table (a type the game doesn't parse keeps its file;
                // one it parses and drops, `minecraft:smelting` with no result, is no recipe).
                assert_eq!(ids("minecraft:stonecutting"), ["minecraft:iron_ingot"]);
                // Not crafted, and no recipe-book meta of their own.
                assert!(recipes.crafting.is_empty() && recipes.cooking.is_empty());
                assert!(!recipes.meta.keys().any(|k| k.starts_with("mymod:")));
                // A pack replaces `mymod:b`, and adds `pack:d` and a `minecraft:p` of its own type.
                let packs = [
                    (
                        "recipe/mymod:b".to_owned(),
                        vec![file("mymod:crusher", "pack-b")],
                    ),
                    ("recipe/pack:d".to_owned(), vec![file("mymod:crusher", "d")]),
                    (
                        "recipe/minecraft:iron_ingot".to_owned(),
                        vec![cutting("pack-iron")],
                    ),
                    (
                        "recipe/minecraft:p".to_owned(),
                        vec![file("pack:press", "p")],
                    ),
                ];
                let recipes = Recipes::load(
                    &root,
                    &items,
                    &PackOverlay::new(&packs),
                    &crate::test_resources::server_load(),
                )
                .unwrap();
                let crusher = &recipes.by_type["mymod:crusher"];
                assert_eq!(
                    crusher.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
                    ["mymod:a", "mymod:b", "pack:d"]
                );
                assert_eq!(crusher[1].json["tag"], "pack-b");
                assert_eq!(recipes.by_type["pack:press"][0].id, "minecraft:p");
                assert_eq!(
                    recipes.by_type["minecraft:stonecutting"][0].json["tag"],
                    "pack-iron"
                );
                let _ = fs::remove_dir_all(&root);
            },
        );
    }
    #[test]
    fn a_recipe_result_needs_an_item_and_a_sane_count() {
        let result = |json: &str| recipe_result(&serde_json::from_str(json).unwrap());
        let stack = result(r#"{"id":"minecraft:iron_ingot","count":2}"#).unwrap();
        assert_eq!((&*stack.item, stack.count), ("minecraft:iron_ingot", 2));
        assert_eq!(result(r#"{"id":"minecraft:iron_ingot"}"#).unwrap().count, 1);
        assert!(result(r#"{"count":2}"#).is_none());
        assert!(result(r#"{"id":"minecraft:iron_ingot","count":0}"#).is_none());
    }
    #[test]
    fn a_recipe_result_can_be_a_bare_item_id() {
        // `ItemStackTemplate.CODEC` falls back to `Item.CODEC`: `"result": "mod:item"` is one item.
        let result = |json: &str| recipe_result(&serde_json::from_str(json).unwrap());
        let stack = result(r#""ruby:ruby""#).unwrap();
        assert_eq!((&*stack.item, stack.count), ("ruby:ruby", 1));
        // The `item` key of older versions is no `id`, and a number names no item.
        assert!(result(r#"{"item":"ruby:ruby"}"#).is_none());
        assert!(result("42").is_none());
    }
    #[test]
    fn a_shapeless_recipe_makes_the_item_its_bare_result_names() {
        let root = std::env::temp_dir().join(format!("horizon-bare-result-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        fs::write(
            root.join("data/minecraft/recipe/bare.json"),
            br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":"minecraft:stick"}"#,
        )
        .unwrap();
        let items = Items::for_tests(HashMap::new());
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let grid = [Some(ItemStack::new("minecraft:flint", 1)), None, None, None];
        let (_, made) = recipes.find(lc(), &grid, 2).unwrap();
        assert_eq!((&*made.item, made.count), ("minecraft:stick", 1));
        let _ = fs::remove_dir_all(&root);
    }

    /// ResourceManagerRegistryLoadTask registers by resource-file Identifier,
    /// and RecipeMap keeps that order when taking the first matching recipe,
    /// for the crafting grid and the furnaces alike.
    #[test]
    fn recipe_selection_follows_resource_file_order() {
        let root =
            std::env::temp_dir().join(format!("horizon-recipe-order-{}", std::process::id()));
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = Items::for_tests(HashMap::new());
        let crafting = |item: &str| {
            serde_json::json!({
                "type": "minecraft:crafting_shapeless", "ingredients": ["minecraft:flint"],
                "result": {"id": item}
            })
        };
        let smelting = |item: &str| {
            serde_json::json!({
                "type": "minecraft:smelting", "ingredient": "minecraft:flint",
                "result": {"id": item}
            })
        };
        let flint = ItemStack::new("minecraft:flint", 1);
        let cases = [
            (vec!["minecraft:z_pick", "parity:a_pick"], "parity:a_pick"),
            (
                vec!["a_priority:z_pick", "z_priority:a_pick"],
                "z_priority:a_pick",
            ),
            (
                vec!["z_priority:equal", "a_priority:equal"],
                "a_priority:equal",
            ),
            // File names include .json: '-' sorts before the parent's '.'.
            (
                vec!["parity:prefix", "parity:prefix-early"],
                "parity:prefix-early",
            ),
            (vec!["z_pick", "parity:a_pick"], "parity:a_pick"),
        ];
        let kinds: [(&str, &dyn Fn(&str) -> serde_json::Value); 2] =
            [("crafting", &crafting), ("smelting", &smelting)];
        for (ids, expected) in cases {
            for (kind, recipe) in kinds {
                for reverse in [false, true] {
                    let mut sources: Vec<_> = ids
                        .iter()
                        .map(|id| (id.to_string(), recipe("minecraft:stick")))
                        .collect();
                    if reverse {
                        sources.reverse();
                    }
                    let recipes = Recipes::from_sources(
                        &root,
                        &items,
                        &PackOverlay::NONE,
                        sources,
                        &crate::test_resources::server_load(),
                    )
                    .unwrap();
                    let (id, result) = match kind {
                        "crafting" => recipes.find(lc(), &[Some(flint.clone())], 1).unwrap(),
                        _ => {
                            let c = recipes.cook(lc(), "minecraft:smelting", &flint).unwrap();
                            (c.id.as_str(), c.result.clone())
                        }
                    };
                    assert_eq!(
                        id, expected,
                        "{kind}, input order: {ids:?}, reversed={reverse}"
                    );
                    assert_eq!(&*result.item, "minecraft:stick");
                }
            }
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// A recipe whose `neoforge:conditions` hold loads, one whose don't is
    /// silently absent, from the root's own files and a pack's alike.
    #[test]
    fn load_conditions_gate_recipes() {
        mod_platform::testing::with_recipe_types(&["mymod:crusher"], || {
            let root =
                std::env::temp_dir().join(format!("horizon-conditions-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
            fs::write(
                root.join(horizon_items::content_namespaces::MARKER),
                br#"["mymod"]"#,
            )
            .unwrap();
            fs::write(
                root.join(horizon_mod_package::enabled::MARKER),
                br#"{"mymod":"both"}"#,
            )
            .unwrap();
            let file = |conditions: &str, tag: &str| {
                format!(
                    r#"{{"type":"mymod:crusher","tag":"{tag}","neoforge:conditions":{conditions}}}"#
                )
                .into_bytes()
            };
            // `mymod` is in the root, `ic2` is not.
            fs::write(
                root.join("data/minecraft/recipe/yes.json"),
                file(r#"[{"type":"neoforge:mod_loaded","modid":"mymod"}]"#, "yes"),
            )
            .unwrap();
            fs::write(
                root.join("data/minecraft/recipe/no.json"),
                file(r#"[{"type":"neoforge:mod_loaded","modid":"ic2"}]"#, "no"),
            )
            .unwrap();
            fs::write(
            root.join("data/minecraft/recipe/inverted.json"),
            file(
                r#"[{"type":"neoforge:not","value":{"type":"neoforge:mod_loaded","modid":"ic2"}}]"#,
                "inverted",
            ),
        )
        .unwrap();
            let items = Items::for_tests(HashMap::new());
            let recipes = Recipes::load(
                &root,
                &items,
                &PackOverlay::NONE,
                &crate::test_resources::server_load(),
            )
            .unwrap();
            let ids = &recipes.by_type["mymod:crusher"];
            assert_eq!(
                ids.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
                ["minecraft:inverted", "minecraft:yes"]
            );
            // A pack's gated file answers the same gate.
            let packs = [(
                "recipe/minecraft:pack_yes".to_owned(),
                vec![file(r#"[{"type":"neoforge:true"}]"#, "pack_yes")],
            )];
            let recipes = Recipes::load(
                &root,
                &items,
                &PackOverlay::new(&packs),
                &crate::test_resources::server_load(),
            )
            .unwrap();
            let ids = &recipes.by_type["mymod:crusher"];
            assert!(ids.iter().any(|r| r.id == "minecraft:pack_yes"));
            let _ = fs::remove_dir_all(&root);
        });
    }
    /// A pack's recipe filter removes a recipe and swaps a mod recipe's input
    /// in the loaded tables, after the packs are merged.
    #[test]
    fn recipe_filters_remove_and_swap() {
        mod_platform::testing::with_recipe_types(&["mymod:crusher"], || {
            let root = std::env::temp_dir().join(format!("horizon-filters-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
            let crusher = |input: &str| {
                format!(
                    r#"{{"type":"mymod:crusher","input":"{input}","result":{{"id":"mymod:dust"}}}}"#
                )
                .into_bytes()
            };
            fs::write(
                root.join("data/minecraft/recipe/iron.json"),
                crusher("minecraft:iron_ingot"),
            )
            .unwrap();
            fs::write(
                root.join("data/minecraft/recipe/gold.json"),
                crusher("minecraft:gold_ingot"),
            )
            .unwrap();
            let filter = br##"{"filters":[
            {"id":"minecraft:gold","remove":true},
            {"type":"mymod:*","replace_input":{"from":"minecraft:iron_ingot","to":"#c:ingots/iron"}}
        ]}"##;
            let packs = [("recipe_filter/pack:f".to_owned(), vec![filter.to_vec()])];
            let items = Items::for_tests(HashMap::new());
            let recipes = Recipes::load(
                &root,
                &items,
                &PackOverlay::new(&packs),
                &crate::test_resources::server_load(),
            )
            .unwrap();
            let crushed = &recipes.by_type["mymod:crusher"];
            assert_eq!(
                crushed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
                ["minecraft:iron"]
            );
            assert_eq!(crushed[0].json["input"], "#c:ingots/iron");
            let _ = fs::remove_dir_all(&root);
        });
    }
    #[test]
    fn cook_finds_the_first_recipe_the_scan_finds() {
        let recipes = cooking_book();
        for kind in [
            "smelting",
            "blasting",
            "smoking",
            "campfire_cooking",
            "other",
        ] {
            for n in 0..420 {
                let stack = ItemStack::new(&format!("minecraft:item_{n}"), 1);
                let a = recipes.cook(lc(), kind, &stack).map(|c| &c.id);
                let b = recipes.cook_scan(lc(), kind, &stack).map(|c| &c.id);
                assert_eq!(a, b, "{kind} item_{n}");
            }
        }
    }
    /// A furnace's recipe lookup, once per lit furnace per tick.
    #[test]
    #[ignore]
    fn cook_cost() {
        let recipes = cooking_book();
        let stacks: Vec<ItemStack> = (0..1000)
            .map(|n| ItemStack::new(&format!("minecraft:item_{}", n % 400), 1))
            .collect();
        let t = std::time::Instant::now();
        let a = stacks
            .iter()
            .filter(|s| recipes.cook_scan(lc(), "smelting", s).cloned().is_some())
            .count();
        let before = t.elapsed().as_secs_f64() * 1e3;
        recipes.cook(lc(), "smelting", &stacks[0]);
        let t = std::time::Instant::now();
        let b = stacks
            .iter()
            .filter(|s| recipes.cook(lc(), "smelting", s).is_some())
            .count();
        let after = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(a, b);
        println!("1000 furnace lookups (scan and clone -> index): {before:.3} ms -> {after:.3} ms");
    }
    fn run(special: &Special, cells: &[ItemStack]) -> Option<ItemStack> {
        let recipes = Recipes {
            mod_read_only: Vec::new(),
            mod_crafting: false,
            crafting: Vec::new(),
            cooking: Vec::new(),
            by_type: HashMap::new(),
            meta: HashMap::new(),
            facts: HashMap::new(),
            curses: HashSet::new(),
            cook_index: Default::default(),
            property_additions: PropertyAdditions::default(),
        };
        let ctx = Context {
            recipes: &recipes,
            context: lc(),
            map_scale: &|_| None,
        };
        let mut grid: Vec<Option<ItemStack>> = cells.iter().cloned().map(Some).collect();
        grid.resize(9, None);
        let input = Input::of(&grid, 3)?;
        special
            .matches(&ctx, &input)
            .then(|| special.assemble(&ctx, &input))
            .flatten()
    }
    #[test]
    fn banner_duplicate_copies_the_patterns_of_the_same_colour() {
        let recipe = Special::BannerDuplicate {
            banner: ingredient(&["minecraft:white_banner"]),
            result: Template::parse(&serde_json::json!("minecraft:white_banner")).unwrap(),
        };
        let source = patterned("minecraft:white_banner");
        let blank = ItemStack::new("minecraft:white_banner", 1);
        let out = run(&recipe, &[source.clone(), blank.clone()]).expect("copy");
        assert_eq!(out.data().unwrap().banner_patterns.len(), 1);
        // Two blanks, or two patterned banners, do not match.
        assert!(run(&recipe, &[blank.clone(), blank.clone()]).is_none());
        assert!(run(&recipe, &[source.clone(), source.clone()]).is_none());
        // Another colour is not in the recipe's ingredient.
        let other = ItemStack::new("minecraft:red_banner", 1);
        assert!(run(&recipe, &[source, other]).is_none());
    }
    #[test]
    fn shield_decoration_replaces_component_removals() {
        let recipe = Special::ShieldDecoration {
            banner: ingredient(&["minecraft:red_banner"]),
            target: ingredient(&["minecraft:shield"]),
            result: Template::parse(&serde_json::json!({"id":"minecraft:shield","components":{"!minecraft:base_color":{},"!minecraft:banner_patterns":{}}})).unwrap(),
        };
        let out = run(
            &recipe,
            &[
                patterned("minecraft:red_banner"),
                ItemStack::new("minecraft:shield", 1),
            ],
        )
        .unwrap();
        let patch = crate::item_data::encode_components(&out).unwrap();
        assert_eq!(
            patch.get("minecraft:base_color"),
            Some(&Tag::String("red".into()))
        );
        assert!(patch.get("minecraft:banner_patterns").is_some());
        assert!(patch.get("!minecraft:base_color").is_none());
        assert!(patch.get("!minecraft:banner_patterns").is_none());
    }
    #[test]
    fn shield_decoration_needs_a_bare_shield() {
        let recipe = Special::ShieldDecoration {
            banner: ingredient(&["minecraft:red_banner"]),
            target: ingredient(&["minecraft:shield"]),
            result: Template::parse(&serde_json::json!("minecraft:shield")).unwrap(),
        };
        let banner = patterned("minecraft:red_banner");
        let shield = ItemStack::new("minecraft:shield", 1);
        let out = run(&recipe, &[banner.clone(), shield]).expect("decorated");
        let data = out.data().unwrap();
        assert_eq!(data.base_color.as_deref(), Some("red"));
        assert_eq!(data.banner_patterns.len(), 1);
        // A shield that already has patterns cannot be decorated again.
        assert!(run(&recipe, &[banner, out]).is_none());
    }
    #[test]
    fn shapeless_matching_assigns_each_stack_one_ingredient() {
        let any = ingredient(&["a", "b"]);
        let only_a = ingredient(&["a"]);
        let (a, b) = (ItemStack::new("a", 1), ItemStack::new("b", 1));
        // Greedy matching would give `a` to the first ingredient and fail.
        assert!(matches_all(lc(), &[any.clone(), only_a.clone()], &[&a, &b]));
        assert!(!matches_all(lc(), &[only_a.clone(), only_a], &[&a, &b]));
        assert!(!matches_all(lc(), &[any], &[&a, &b]));
    }
    #[test]
    fn dyes_blend_like_dyed_item_color() {
        // One red dye on an undyed item gives the dye's own colour.
        assert_eq!(apply_dyes(None, &[14]), 0xB02E26);
    }
}

#[cfg(test)]
mod crafting_remainder_tests {
    use super::*;
    #[test]
    fn crafting_remainder_book_copy_preserves_generation_and_components() {
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "copy".into(),
            Recipe::Special(Special::BookCloning {
                source: Ingredient::of(vec!["minecraft:written_book".into()]),
                material: Ingredient::of(vec!["minecraft:writable_book".into()]),
                min: 1,
                max: 2,
                result: Template {
                    item: Some("minecraft:written_book".into()),
                    count: 1,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            }),
        ));
        let mut book = ItemStack::new("minecraft:written_book", 1);
        book.data_mut().written = Some(crate::item_data::WrittenBook {
            title: "Original".into(),
            author: "Author".into(),
            generation: 1,
            resolved: false,
            pages: vec!["page".into()],
            page_tags: Vec::new(),
        });
        let grid = [
            Some(book.clone()),
            Some(ItemStack::new("minecraft:writable_book", 1)),
            None,
            None,
        ];
        let calls = std::cell::Cell::new(0);
        let remainder = recipes.remainders("copy", &grid, 2, &|_| {
            calls.set(calls.get() + 1);
            None
        });
        assert_eq!(
            calls.get(),
            1,
            "the source book's recipe copy ends the search"
        );
        assert_eq!(remainder, vec![Some(book), None, None, None]);
        let written = remainder[0]
            .as_ref()
            .unwrap()
            .data()
            .unwrap()
            .written
            .as_ref()
            .unwrap();
        assert_eq!(written.generation, 1);
        assert_eq!(written.pages, ["page"]);
    }
}
