//! Custom book identities, independent of recipe categories and vanilla ordinals.

use crate::item::recipe_book::RecipeBookTypeKey;

/// Operations available during the existing shared mod registration phase.
pub trait RecipeBooksRegistrar {
    /// Register an explicit lowercase `namespace:path` in any manifest-owned namespace.
    /// IDs are limited to 256 bytes and 1024 books in total. Invalid or duplicate
    /// declarations refuse installation even if the caller ignores the error.
    /// The registry freezes when registration ends, before player state is loaded.
    fn recipe_book_type(&mut self, id: &str) -> Result<RecipeBookTypeKey, String>;
}
