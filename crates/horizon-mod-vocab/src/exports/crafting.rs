mod __private {
    pub use crate::__private::ingredient_parser::{
        WalkedIngredient, dedup_keep_order, resolved_items,
    };
}

pub use crate::item::crafting::{
    Ingredient, IngredientKeysError, Recipe, RecipeInput, RecipeLookupError, RecipeMatch,
    RecipePropertyAddition,
};
