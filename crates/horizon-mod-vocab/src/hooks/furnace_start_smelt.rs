//! The `furnace_start_smelt` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0223

pub trait Hooks {
    /// A furnace at `pos` began cooking `item` (its cooking timer advanced
    /// from zero). Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_start_smelt();
}
