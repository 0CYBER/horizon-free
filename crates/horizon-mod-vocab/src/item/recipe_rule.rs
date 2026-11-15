//! Typed rules for recipes whose result is not a fixed crafting stack.

use crate::{Ingredient, ItemStack, Tag};
/// A firework explosion's shape: what a firework recipe selects and a
/// [`crate::FireworkExplosion`] draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FireworkShape {
    /// Small spherical explosion.
    #[default]
    SmallBall,
    /// Large spherical explosion.
    LargeBall,
    /// Star-shaped explosion.
    Star,
    /// Creeper-face explosion.
    Creeper,
    /// Burst explosion.
    Burst,
}
impl FireworkShape {
    /// The component's serialized shape name.
    pub fn name(self) -> &'static str {
        match self {
            Self::SmallBall => "small_ball",
            Self::LargeBall => "large_ball",
            Self::Star => "star",
            Self::Creeper => "creeper",
            Self::Burst => "burst",
        }
    }
    /// Decode a component shape name; vanilla uses the small ball for unknown names.
    pub fn from_name(name: &str) -> Self {
        match name {
            "large_ball" => Self::LargeBall,
            "star" => Self::Star,
            "creeper" => Self::Creeper,
            "burst" => Self::Burst,
            _ => Self::SmallBall,
        }
    }
}
#[cfg(test)]
mod shape_tests {
    use super::FireworkShape::*;
    #[test]
    fn firework_shape_case_order() {
        assert_eq!(
            [
                SmallBall as u8,
                LargeBall as u8,
                Star as u8,
                Creeper as u8,
                Burst as u8
            ],
            [0, 1, 2, 3, 4]
        );
    }
}

/// An `ItemStackTemplate` in a crafting rule, not an assembled result.
/// The rule decides which input supplies inherited components and, when
/// `item` is absent, the result item. Use [`crate::LevelRef::recipe_for`] for an
/// actual grid's output and remainders.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
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
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
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
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
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
/// results are assembled by [`crate::LevelRef::recipe_for`], not by treating the
/// template as a fixed stack. A mod-defined recipe type keeps its raw JSON
/// instead of inventing a vanilla rule.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
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

/// Refusal when decoding a recipe-file field.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum RecipeJsonError {
    /// The text is not syntactically valid JSON.
    InvalidJson,
    /// The resolved result cannot be represented by the API stack boundary.
    InvalidStack(crate::StackWriteError),
}
impl From<crate::StackWriteError> for RecipeJsonError {
    fn from(error: crate::StackWriteError) -> Self {
        Self::InvalidStack(error)
    }
}
