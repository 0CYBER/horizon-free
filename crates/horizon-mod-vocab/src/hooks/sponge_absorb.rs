//! The `sponge_absorb` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0332

pub trait Hooks {
    /// A sponge at `pos` absorbed water and became wet. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_sponge_absorb"
    )]
    fn sponge_absorb();
}
