//! The `player_toggle_entity_age_lock` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0316

pub trait Hooks {
    /// A server player toggled an ageable mob's age lock with a golden
    /// dandelion, after the lock and item use changed.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_toggle_entity_age_lock();
}
