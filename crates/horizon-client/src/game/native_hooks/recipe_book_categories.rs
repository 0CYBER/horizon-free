use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Declare categories for custom recipe types, with registered item icons.
    /// Asked lazily once per recipe table by `recipe_book_category`.
    /// The complete batch is validated atomically; any invalid declaration
    /// refuses the generation. Multiple categories may share a recipe type.
    /// This does not yet add tabs or book displays.
    pub(in crate::game) fn native_recipe_book_categories(
        &mut self,
        categories: &mut Vec<RecipeBookCategory>,
    ) {
        self.fire_recipe_book_categories(categories);
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The validated category and icon of a custom recipe type, or None for
    /// an undeclared/invalid type, before resources load, or outside a world.
    /// Returns the first category in category-id order when a type has several.
    /// Any invalid declaration refuses the complete generation.
    /// Rebuilt when the recipe table changes. Does not unlock any recipes.
    pub(in crate::game) fn recipe_book_category(
        &mut self,
        kind: &str,
    ) -> Option<RecipeBookCategory> {
        if self.outside_world() || self.res.rest.get().is_none() {
            return None;
        }
        self.registered_recipe_book_category(kind)
    }
}
