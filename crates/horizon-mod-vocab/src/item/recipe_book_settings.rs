//! The authoritative pair owned by one player's registered custom book.

/// Open and craftable-only state, independent of categories and vanilla books.
/// Client intent must not be saved until the server accepts it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: two boolean flags are the complete settings pair
pub struct RecipeBookSettings {
    /// Whether the book panel is open.
    pub open: bool,
    /// Whether only craftable recipes are shown.
    pub filtering: bool,
}

/// Typed identity reported at the one settings-change moment.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: vanilla identities and a registered custom identity
pub enum RecipeBookIdentity {
    Crafting,
    Furnace,
    BlastFurnace,
    Smoker,
    Custom(crate::RecipeBookTypeKey),
}
