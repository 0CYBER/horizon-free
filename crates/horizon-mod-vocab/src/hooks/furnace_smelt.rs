//! The `furnace_smelt` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0222

pub trait Hooks {
    /// A furnace finished smelting `item` into `result` at `pos`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_smelt();
}
