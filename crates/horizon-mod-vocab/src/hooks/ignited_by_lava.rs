//! The `ignited_by_lava` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, whether lava sets fire beside the block
    /// `state` at `pos` (`LavaFluid.isFlammable`, the only reader of
    /// `BlockStateBase.ignitedByLava`), asked from its side `face`, the one
    /// that faces the air the fire would take, as lava's random tick looks
    /// for something to light. `Some(true)` lets lava light it, `Some(false)`
    /// refuses it, and left `None` the block's own flag answers, as vanilla's
    /// does (planks, wool and leaves yes; stone no). Answer only for your own
    /// blocks. The level is read-only.
    #[hook(
        side = "server",
        contract = "replace",
        out = "ignites",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::ignited_by_lava::GameLevel<'_>::native_ignited_by_lava",
        neoforge = "IBlockExtension.ignitedByLava"
    )]
    fn ignited_by_lava();
}
