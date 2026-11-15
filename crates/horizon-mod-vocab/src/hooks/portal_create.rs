//! The `portal_create` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0321

pub trait Hooks {
    /// A Nether portal was built for a dimension travel, or an End platform
    /// was made for an arrival. `pos` is the lower corner of the Nether
    /// portal rectangle or the End platform origin. The completed blocks are
    /// already in the world.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn portal_create();
}
