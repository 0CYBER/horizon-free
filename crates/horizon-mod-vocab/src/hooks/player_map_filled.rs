//! The `player_map_filled` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0298

pub trait Hooks {
    /// A server player filled an empty map, after the filled map and its saved data were created.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_map_filled();
}
