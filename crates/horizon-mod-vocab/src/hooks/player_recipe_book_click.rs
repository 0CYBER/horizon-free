//! The `player_recipe_book_click` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0304

pub trait Hooks {
    /// A server player's recipe book requested a known recipe be placed in
    /// their crafting grid. `recipe` is the recipe id; `all` is the shift
    /// click form. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_recipe_book_click();
}
