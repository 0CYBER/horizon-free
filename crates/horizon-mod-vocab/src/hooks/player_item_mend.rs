//! The `player_item_mend` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0256

pub trait Hooks {
    /// Mending repaired a server player's item with experience from an orb. `repair` is durability
    /// restored and `experience` is the orb's value. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_item_mend();
}
