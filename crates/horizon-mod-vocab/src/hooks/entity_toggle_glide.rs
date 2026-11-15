//! The `entity_toggle_glide` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0209

pub trait Hooks {
    /// A guest's fall flying state changed on the server. The change has
    /// already been applied.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_toggle_glide();
}
