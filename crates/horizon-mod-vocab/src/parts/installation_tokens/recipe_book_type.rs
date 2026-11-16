//! A book identity validated by the owning registrar.
/// Construct the identity before its lookup table is frozen.
pub fn recipe_book_type(id: String) -> crate::RecipeBookTypeKey {
    crate::RecipeBookTypeKey { id }
}
