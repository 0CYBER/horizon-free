//! The `block_shear_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0181

pub trait Hooks {
    /// A dispenser sheared `entity` of kind `kind` at the block in front of
    /// its own block at `pos`, after the shearing effects. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_shear_entity();
}
