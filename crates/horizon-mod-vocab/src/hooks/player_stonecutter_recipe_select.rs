//! The `player_stonecutter_recipe_select` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0311

pub trait Hooks {
    /// A server player selected a stonecutter recipe, after the menu accepted
    /// its button. `recipe` is the zero-based button index. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_stonecutter_recipe_select();
}
