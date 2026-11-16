//! The `block_spread` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0040

pub trait Hooks {
    /// On an authoritative server, a fluid flows or fire spreads from `from`
    /// into `to` (`kind` is `fluid` or `fire`): `FlowingFluid.spreadTo`, and
    /// `FireBlock.tick` setting fire to `to` or burning it (`checkBurnOut`).
    /// Set `canceled` to leave the block at `to` as it is. Fluids ask on
    /// every flow: answer fast.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_spread",
        neoforge = "none (BlockEvent.FluidPlaceBlockEvent covers only fluids making stone and the like, not flow or fire spread)"
    )]
    fn block_spread();
}
