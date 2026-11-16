//! The `recipe_book_categories` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Supply categories and vanilla-shaped displays for custom recipe types.
    /// Asked lazily once per recipe table. Use the Host's recipe table to build
    /// `RecipeBookCategory::displays` from the type's own opaque recipe JSON.
    /// Each category names its menu, book identity and per-book search group.
    /// The display/index load boundary returns typed errors; screen integration
    /// is separate. This declaration does not register a settings book.
    #[hook(
        side = "both",
        contract = "listen",
        neoforge = "none (RegisterRecipeBookSearchCategoriesEvent registers search groups only, not a recipe type's categories)"
    )]
    fn recipe_book_categories();
}
