use serde_json::Value;
use std::sync::Arc;
// Engine-owned recipe snapshots.

use super::{Ingredient, ItemStack};
use horizon_core::nbt::Tag;
pub use horizon_items::firework::FireworkShape;

/// An `ItemStackTemplate` in a crafting rule, not an assembled result.
/// The rule decides which input supplies inherited components and, when
/// `item` is absent, the result item. Use [`crate::LevelRef::craft`] for an
/// actual grid's output and remainders.
#[derive(Clone, Debug)]
pub struct RecipeTemplate {
    /// Replacement item, or `None` to keep the rule's input item.
    pub item: Option<String>,
    /// Template count, before any material-count addition.
    pub count: u32,
    /// Component patch applied after inherited components, including explicit
    /// removals (`!component`) and zero values; `None` for no patch.
    pub components: Option<Tag>,
}
impl RecipeTemplate {
    /// A template with its parsed replacement item, count and component patch.
    pub fn new(item: Option<String>, count: u32, components: Option<Tag>) -> Self {
        Self {
            item,
            count,
            components,
        }
    }
}

/// An ingredient that chooses a firework star's explosion shape.
#[derive(Clone, Debug)]
pub struct FireworkShapeIngredient {
    /// Shape selected by this ingredient.
    pub shape: FireworkShape,
    /// Items which select the shape.
    pub ingredient: Ingredient,
}
impl FireworkShapeIngredient {
    /// A parsed shape and its ingredient.
    pub fn new(shape: FireworkShape, ingredient: Ingredient) -> Self {
        Self { shape, ingredient }
    }
}

/// A trim pattern registry reference or an inline pattern definition.
#[derive(Clone, Debug)]
pub enum RecipeTrimPattern {
    /// Registry identifier.
    Reference(String),
    /// Inline registry value, including its text component.
    Inline {
        /// Asset identifier.
        asset_id: String,
        /// Description text component in its JSON data form.
        description: Tag,
        /// Whether the pattern is a decal.
        decal: bool,
    },
}

/// The parsed rule of a stonecutting, smithing or special crafting recipe.
/// These are the same resolved ingredients and rules the game uses, not a
/// second recipe implementation. Only `Stonecutting` has a fixed output;
/// every other variant depends on the grid or smithing inputs (components,
/// counts, dyes, trim material, damage or map state). Special crafting
/// results are assembled by [`crate::LevelRef::craft`], not by treating the
/// template as a fixed stack. A mod-defined recipe type keeps its raw JSON
/// instead of inventing a vanilla rule.
#[derive(Clone, Debug)]
pub enum RecipeRule {
    /// A stonecutter input and fixed result.
    Stonecutting {
        /// Accepted input.
        input: Ingredient,
        /// Exact output stack.
        result: ItemStack,
    },
    /// Apply a result template over the base item and its components.
    SmithingTransform {
        /// Optional template slot; absent requires an empty slot.
        template: Option<Ingredient>,
        /// Base slot; supplies the output components.
        base: Ingredient,
        /// Optional addition slot; absent requires an empty slot.
        addition: Option<Ingredient>,
        /// Result template applied to inherited components.
        result: RecipeTemplate,
    },
    /// Apply the pattern using the addition's trim material to the base.
    SmithingTrim {
        /// Template slot.
        template: Ingredient,
        /// Base slot; supplies the output item and components.
        base: Ingredient,
        /// Addition slot; selects the trim material.
        addition: Ingredient,
        /// Registry reference or inline trim pattern.
        pattern: RecipeTrimPattern,
    },
    /// Copy a patterned banner onto a blank banner of the same colour.
    BannerDuplicate {
        /// Banner ingredient.
        banner: Ingredient,
        /// Result template; patterns come from the grid.
        result: RecipeTemplate,
    },
    /// Apply a banner's patterns to an undecorated shield.
    ShieldDecoration {
        /// Banner supplying patterns and colour.
        banner: Ingredient,
        /// Shield to decorate.
        target: Ingredient,
        /// Result template; components come from the grid.
        result: RecipeTemplate,
    },
    /// One input and a bounded number of materials, retaining input components.
    Transmute {
        /// Input supplying the inherited item and components.
        input: Ingredient,
        /// Material ingredient.
        material: Ingredient,
        /// Minimum number of occupied material slots.
        min: usize,
        /// Maximum number of occupied material slots.
        max: usize,
        /// Add the number of materials to the template's result count.
        add_count: bool,
        /// Patch applied after the input's components.
        result: RecipeTemplate,
    },
    /// Dye a target using the grid's dye colours.
    Dye {
        /// Target item.
        target: Ingredient,
        /// Dye items.
        dye: Ingredient,
        /// Template; the resulting colour depends on the grid.
        result: RecipeTemplate,
    },
    /// Imbue materials with the source's potion contents.
    Imbue {
        /// Source of potion contents.
        source: Ingredient,
        /// Material around the source.
        material: Ingredient,
        /// Template receiving the potion contents.
        result: RecipeTemplate,
    },
    /// Copy a written book within the allowed generation range.
    BookCloning {
        /// Written book supplying contents and generation.
        source: Ingredient,
        /// Blank books.
        material: Ingredient,
        /// Minimum allowed source generation.
        min: i32,
        /// Maximum allowed source generation.
        max: i32,
        /// Template; count and book contents depend on the grid.
        result: RecipeTemplate,
    },
    /// Build rockets with flight duration and explosions from the grid.
    FireworkRocket {
        /// Rocket shell.
        shell: Ingredient,
        /// Fuel, whose occupied slots choose flight duration.
        fuel: Ingredient,
        /// Optional stars supplying explosions.
        star: Ingredient,
        /// Rocket template.
        result: RecipeTemplate,
    },
    /// Build a star with colours, shape and effects from the grid.
    FireworkStar {
        /// Colour ingredients.
        dye: Ingredient,
        /// Fuel ingredient.
        fuel: Ingredient,
        /// Optional trail ingredient.
        trail: Ingredient,
        /// Optional twinkle ingredient.
        twinkle: Ingredient,
        /// Shape ingredients, in the parsed rule's order.
        shapes: Vec<FireworkShapeIngredient>,
        /// Star template.
        result: RecipeTemplate,
    },
    /// Add fade colours to an existing star.
    FireworkStarFade {
        /// Fade colour ingredients.
        dye: Ingredient,
        /// Star whose other explosion properties are retained.
        target: Ingredient,
        /// Star template.
        result: RecipeTemplate,
    },
    /// Extend a map when its live map state permits it.
    MapExtending {
        /// Map input.
        map: Ingredient,
        /// Surrounding material.
        material: Ingredient,
        /// Template; map identity and postprocessing depend on the input.
        result: RecipeTemplate,
    },
    /// Build a pot with decorations from the four sides of the grid.
    DecoratedPot {
        /// Back decoration.
        back: Ingredient,
        /// Left decoration.
        left: Ingredient,
        /// Right decoration.
        right: Ingredient,
        /// Front decoration.
        front: Ingredient,
        /// Pot template.
        result: RecipeTemplate,
    },
    /// Repair two damaged items of the same kind; eligibility, damage and
    /// retained curses depend on the input stacks, not a fixed ingredient list.
    RepairItem,
}

/// A recipe in force (`RecipeManager.getAllRecipesFor`), whatever its type
/// (vanilla's or a mod's own): its typed parts, and its file for whatever a
/// mod's own type keeps there. Plain data.
///
/// A recipe the game could not parse (an ingredient it cannot read, no
/// result) is not in force and is not listed.
///
/// `ingredients` and `results` are filled from the game's own parse of the
/// shaped, shapeless and cooking types: a shaped recipe's pattern cells row
/// by row (empty cells left out), a shapeless recipe's list, a cooking
/// recipe's `ingredient`, and the result as `results[0]`. Stonecutting,
/// smithing and special crafting recipes instead expose their typed [`RecipeRule`]
/// in `rule`, keeping these two lists empty. Dynamic outputs are rules/templates,
/// not fixed stacks; use [`LevelRef::craft`] to assemble a crafting grid.
/// A mod's own type leaves these fields empty and keeps its `json`.
#[derive(Clone, Debug)]
pub struct RecipeSnapshot {
    /// `<namespace>:<path>`, its place under `data/<namespace>/recipe/`.
    pub id: String,
    /// The recipe type, `minecraft:smelting` or `mymod:crusher`.
    pub kind: String,
    /// The recipe book group, empty for none.
    pub group: String,
    /// What it takes, as above.
    pub ingredients: Vec<Ingredient>,
    /// What it makes, as above.
    pub results: Vec<ItemStack>,
    /// Ticks a cooking recipe (`smelting`, `blasting`, `smoking`,
    /// `campfire_cooking`) takes, its file's `cookingtime`; `None` for any
    /// other type.
    pub cooking_time: Option<u32>,
    /// Experience a cooking recipe gives, `None` for any other type.
    pub experience: Option<f32>,
    /// Parsed stonecutting, smithing or special crafting rule. `None` for
    /// shaped, shapeless, cooking and mod-defined types. The ingredient roles,
    /// bounds and templates come from the game's canonical parse.
    pub rule: Option<RecipeRule>,
    /// The recipe's file, as the data pack wrote it.
    pub json: Arc<Value>,
}
impl RecipeSnapshot {
    /// Required inputs must have enumerable candidates before consumer publication.
    /// The recipe and its stack predicates remain in the effective recipe map.
    pub(crate) fn has_placement_candidates(&self) -> bool {
        let nonempty = |ingredient: &Ingredient| !ingredient.items.is_empty();
        if matches!(
            self.json["type"].as_str().unwrap_or_default(),
            "minecraft:crafting_shaped" | "minecraft:crafting_shapeless"
        ) && self.ingredients.is_empty()
        {
            return false;
        }
        self.ingredients.iter().all(nonempty)
            && self.rule.as_ref().is_none_or(|rule| match rule {
                RecipeRule::Stonecutting { input, .. } => nonempty(input),
                RecipeRule::SmithingTransform {
                    template,
                    base,
                    addition,
                    ..
                } => template
                    .iter()
                    .chain(std::iter::once(base))
                    .chain(addition.iter())
                    .all(nonempty),
                RecipeRule::SmithingTrim {
                    template,
                    base,
                    addition,
                    ..
                } => [template, base, addition].into_iter().all(nonempty),
                RecipeRule::Transmute {
                    input, material, ..
                } => [input, material].into_iter().all(nonempty),
                RecipeRule::Dye { target, dye, .. } => [target, dye].into_iter().all(nonempty),
                RecipeRule::Imbue {
                    source, material, ..
                } => [source, material].into_iter().all(nonempty),
                // Vanilla's other special rules have no learnable book display.
                _ => true,
            })
    }

    /// The recipe `id` of type `kind` with its file `json`, the typed parts empty.
    pub fn new(id: impl Into<String>, kind: impl Into<String>, json: Arc<Value>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            group: String::new(),
            ingredients: Vec::new(),
            results: Vec::new(),
            cooking_time: None,
            experience: None,
            rule: None,
            json,
        }
    }
}
