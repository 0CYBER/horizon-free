//! The `exp_bottle` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0211

pub trait Hooks {
    /// An experience bottle broke and its experience was awarded.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn exp_bottle();
}
