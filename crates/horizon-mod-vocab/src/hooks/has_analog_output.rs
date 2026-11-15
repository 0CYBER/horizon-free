//! The `has_analog_output` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0072

pub trait Hooks {
    /// `BlockState.hasAnalogOutputSignal`: set `output` for a block of yours
    /// that comparators read (how full or far along it is, as a composter's
    /// level), so they ask `analog_output`. Asked from inside the
    /// level's update: answer from `state` alone. Authoritative servers only.
    #[hook(
        side = "server",
        contract = "collect",
        out = "output",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_has_analog_output",
        neoforge = "none (vanilla BlockBehaviour.hasAnalogOutputSignal; NeoForge adds no event or extension for it)"
    )]
    fn has_analog_output();
}
