//! The `entity_portal` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0204

pub trait Hooks {
    /// A server entity finished its portal wait and is beginning to leave
    /// `dimension` through `pos`. The destination follows the portal rules.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_portal();
}
