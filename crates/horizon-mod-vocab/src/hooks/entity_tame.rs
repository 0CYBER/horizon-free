//! The `entity_tame` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0207

pub trait Hooks {
    /// A mob became tamed after its owner and persistent state were set.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_tame();
}
