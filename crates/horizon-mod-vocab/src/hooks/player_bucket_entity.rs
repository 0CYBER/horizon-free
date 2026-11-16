//! The `player_bucket_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0280

pub trait Hooks {
    /// A server player caught a mob in a bucket, after it was removed and the filled bucket was given.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_bucket_entity();
}
