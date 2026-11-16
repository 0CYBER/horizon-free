//! The `trade_select` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0337

pub trait Hooks {
    /// A server player selected trade `index` from `villager` in the trading menu.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn trade_select();
}
