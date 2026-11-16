//! The `brewing_fuel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0183

pub trait Hooks {
    /// A brewing stand at `pos` consumed `fuel` and gained brewing fuel. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn brewing_fuel();
}
