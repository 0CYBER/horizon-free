//! The `perform_bonemeal` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0084

pub trait Hooks {
    /// `BonemealableBlock.performBonemeal` for a block that answered
    /// `is_valid_bonemeal_target` and whose `is_bonemeal_success`
    /// said yes: grow it here, `state` its state at `pos`. The meal is used up
    /// either way. Change the block with the level's
    /// `native_set_block_property`, and schedule with `native_schedule_block_tick`
    /// (`ticks` is the hook's). Authoritative servers only.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_perform_bonemeal",
        neoforge = "none (vanilla BonemealableBlock.performBonemeal; NeoForge adds no event or extension for it)"
    )]
    fn perform_bonemeal();
}
