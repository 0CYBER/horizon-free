//! The `player_flower_pot_manipulate` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0252

pub trait Hooks {
    /// A server player put a plant into a flower pot, after the potted block state was stored and the
    /// item consumed. `plant` is the item id and `pos` the pot position. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_flower_pot_manipulate();
}
