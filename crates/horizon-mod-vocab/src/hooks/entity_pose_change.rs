//! The `entity_pose_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0205

pub trait Hooks {
    /// A guest player's synchronized pose changed from `from` to `to`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_pose_change();
}
