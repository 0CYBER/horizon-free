//! The `is_bonemeal_success` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0083

pub trait Hooks {
    /// `BonemealableBlock.isBonemealSuccess` for a block that answered
    /// `is_valid_bonemeal_target`, after it did: set `success` to grow
    /// it this time. Vanilla draws the level's random here
    /// (`isBonemealSuccess(Level, RandomSource, BlockPos, BlockState)`), so
    /// draw from `native_level_random` to keep vanilla's order.
    /// Authoritative servers only.
    #[hook(
        side = "server",
        contract = "collect",
        out = "success",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_is_bonemeal_success",
        neoforge = "none (vanilla BonemealableBlock.isBonemealSuccess; NeoForge adds no event or extension for it)"
    )]
    fn is_bonemeal_success();
}
