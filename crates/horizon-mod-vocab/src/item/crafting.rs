//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
use serde_json::Value;
use std::collections::HashSet;
#[path = "recipe_property_validation.rs"]
mod recipe_property_validation;
pub(crate) use recipe_property_validation::validate_recipe_properties;
/// Items a mod adds to one of the recipe property sets: the items a furnace,
/// blast furnace, smoker, campfire, smithing table slot or brewing stand slot
/// accepts (`RecipeManager.RECIPE_PROPERTY_SETS`), or a mod-created menu input
/// set. New keys must belong to a namespace owned by the listener's manifest;
/// later listeners may extend sets created by earlier listeners.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct RecipePropertyAddition {
    /// The set's key, such as `minecraft:furnace_input` or `mymod:pot_input`.
    pub set: String,
    /// The item ids to add.
    pub items: Vec<String>,
}
impl RecipePropertyAddition {
    /// `items` added to the set named `set`.
    pub fn new(set: impl Into<String>, items: Vec<String>) -> Self {
        Self {
            set: set.into(),
            items,
        }
    }
}
/// A set of accepted items (expanded from an item, list or tag), and the
/// items in its `HolderSet` order (`Ingredient.display`'s `TagSlotDisplay`).
/// Optional metadata is boxed so nested ingredient reads fit the guest stack budget.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: placement providers and the game construct complete ingredient values
#[derive(horizon_mod_api_macros::Wire)]
pub struct Ingredient {
    /// Full candidate set; non-simple predicates test the concrete stack.
    pub items: HashSet<String>,
    /// Default-stack display/simple-match order; non-simple kinds may have none.
    pub order: Vec<String>,
    /// A mod's own kind, for matching at craft time: the kind and its
    /// NBT compound, as the load-time hook saw them (`ingredient_test`).
    pub custom: Option<(String, horizon_core::nbt::Compound)>,
    /// NeoForge's shapes, with their children parsed: matching a concrete
    /// stack asks each child its own `test` (a mod's kind among them),
    /// as on NeoForge — the flat list above only names items.
    pub children: Option<(Shape, Vec<Self>)>,
    /// A concrete-stack built-in predicate.
    pub rule: Option<Box<IngredientRule>>,
    /// Explicit slot display, independent of membership.
    pub display: Option<Box<horizon_core::nbt::Compound>>,
}
/// Built-in predicates evaluated against the concrete stack.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
// exhaustive: the parser and engine share this closed set of built-in predicates
pub enum IngredientRule {
    /// Exact component patch, with explicit absence requirements.
    Components {
        patch: horizon_core::nbt::Compound,
        removed: Vec<String>,
        strict: bool,
    },
}
/// How a shape's children combine at match time.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum Shape {
    /// Any child matching the stack is enough.
    Compound,
    /// Every child must match it.
    Intersection,
    /// The first child must, and the second must not.
    Difference,
    /// Matching follows the base; presentation uses a separate slot display.
    CustomDisplay,
}
/// Why an ingredient cannot be retained as an exact, component-independent key set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum IngredientKeysError {
    /// A custom predicate may inspect components or context at match time.
    NotEnumerable,
    /// An ingredient name cannot be resolved in the completed item registry.
    UnknownItem,
}
impl crate::NoBackend for IngredientKeysError {
    fn no_backend() -> Self {
        Self::UnknownItem
    }
}
impl Ingredient {
    /// Resolve exact membership once per load/reload for local matching in mod memory.
    /// Plain item lists, expanded tags and standard shapes with no custom descendants
    /// are component-independent. A custom/component predicate anywhere in the tree
    /// returns `NotEnumerable`: the display item list is not an exact predicate (and
    /// for differences containing custom predicates is not even necessarily a superset).
    /// Such predicates remain mod-owned; this method never invokes a match-time hook.
    /// One host call per ingredient at publication; never call this per tick or candidate.
    pub fn item_keys(&self, host: &dyn Host) -> Result<Arc<[crate::ItemKey]>, IngredientKeysError> {
        backend_of(host)
            .ok_or(IngredientKeysError::UnknownItem)?
            .recipe_ingredient_item_keys(self)
    }
    /// The items of a list, in order.
    pub fn of(items: Vec<String>) -> Self {
        Self {
            items: items.iter().cloned().collect(),
            order: items,
            custom: None,
            children: None,
            rule: None,
            display: None,
        }
    }
    /// `Ingredient.test`: whether `stack` fits the candidate list and custom
    /// predicate (`ICustomIngredient.test`). An empty custom list retains the
    /// predicate without item filtering, but cannot be placed by the recipe book.
    /// An empty tag never matches; a host without a game refuses every stack.
    pub fn test(&self, host: &dyn Host, stack: &ItemStack) -> bool {
        backend_of(host).is_some_and(|backend| backend.recipe_ingredient_test(self, stack))
    }
}
/// A recipe in force (`RecipeManager.getAllRecipesFor`), whatever its type
/// (vanilla's or a mod's own): its typed parts, and its file for whatever a
/// mod's own type keeps there. `json` is the data-pack file's JSON, read by a
/// mod at reload; host parsing helpers accept JSON text. Plain data.
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
/// not fixed stacks; use [`LevelRef::recipe_for`] to match and assemble a crafting grid.
/// A mod's own type leaves these fields empty and keeps its `json`.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Recipe {
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
impl Recipe {
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

/// Typed input to the engine's matched recipe lookup. Slot order is significant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum RecipeInput {
    Crafting {
        width: u32,
        height: u32,
        slots: Vec<Option<ItemStack>>,
    },
    Single(ItemStack),
    Smithing {
        template: Option<ItemStack>,
        base: Option<ItemStack>,
        addition: Option<ItemStack>,
    },
}
/// A refusal distinct from a valid input with no matching recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum RecipeLookupError {
    InvalidInput,
    InvalidItem,
    InvalidRecipe,
    UnavailableLevel,
    /// The engine has no matcher for this type. Cache its recipes once per load/reload
    /// through Host::recipes_of_type and match in mod memory, without per-tick crossings.
    NotMatchable,
}
impl crate::NoBackend for RecipeLookupError {
    fn no_backend() -> Self {
        Self::UnavailableLevel
    }
}
/// What the matched recipe assembles; lookup never consumes the input.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct RecipeMatch {
    pub recipe: crate::Identifier,
    pub output: ItemStack,
    /// Remaining items in input slot order after consuming one item per occupied slot.
    pub remainders: Vec<Option<ItemStack>>,
    pub experience: f32,
    /// Recipe-file cooking time; station-specific speed adjustments are not applied here.
    pub processing_time: u32,
}
impl RecipeMatch {
    /// Build the result of an engine match.
    pub fn new(
        recipe: crate::Identifier,
        output: ItemStack,
        remainders: Vec<Option<ItemStack>>,
        experience: f32,
        processing_time: u32,
    ) -> Self {
        Self {
            recipe,
            output,
            remainders,
            experience,
            processing_time,
        }
    }
}

#[cfg(test)]
mod empty_candidate_tests {
    use super::*;

    #[test]
    fn empty_custom_predicate_survives_native_wasm_wire() {
        use crate::wire::{Reader, Wire, Writer};
        let mut ingredient = Ingredient::of(vec![]);
        ingredient.custom = Some((
            "fixture:predicate".into(),
            [("required_damage".into(), horizon_core::nbt::Tag::Int(7))]
                .into_iter()
                .collect(),
        ));
        let mut writer = Writer::default();
        ingredient.put(&mut writer);
        let decoded: Ingredient = Reader::decode(&writer.into_bytes()).unwrap();
        assert_eq!(decoded, ingredient);
        assert!(decoded.items.is_empty());
        assert!(decoded.order.is_empty());
        assert!(decoded.custom.is_some());
    }
}
