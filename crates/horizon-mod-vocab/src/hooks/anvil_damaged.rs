//! The `anvil_damaged` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0166

pub trait Hooks {
    /// An anvil at `pos` took damage, changing its block from `from` to `to`.
    /// `to` is air when the anvil broke. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn anvil_damaged();
}
