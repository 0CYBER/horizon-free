//! The `rail_flexible` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the rail `state` at `pos` may make corners
    /// (`IBaseRailBlockExtension.isFlexibleRail`). `flexible` starts true for a plain rail and
    /// false for the straight ones (powered, detector, activator); set false to keep it
    /// straight at a junction. Asked on an authoritative server as the rail and its neighbours
    /// connect (`RailState`): answer from the state and the position, without touching the
    /// world. With no part attached to the state, the cost is one table lookup. Diverges from
    /// upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "flexible",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::rails::GameLevel<'_>::native_rail_flexible",
        neoforge = "IBaseRailBlockExtension.isFlexibleRail"
    )]
    fn rail_flexible();
}
