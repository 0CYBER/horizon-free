//! The `player_unleash_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0319

pub trait Hooks {
    /// A server player removed an entity's leash, after it was cleared.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_unleash_entity();
}
