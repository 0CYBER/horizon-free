//! The `rail_can_make_slopes` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the rail `state` at `pos` may climb toward a raised rail beside it
    /// (`IBaseRailBlockExtension.canMakeSlopes`). `can` starts true, as for every vanilla rail;
    /// set false to keep it flat as it connects to its neighbours (`RailState`). Asked on an
    /// authoritative server as a rail is placed or a neighbour changes it: answer from the
    /// state and the position, without touching the world. With no part attached to the
    /// state, the cost is one table lookup. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "can",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::rails::GameLevel<'_>::native_rail_can_make_slopes",
        neoforge = "IBaseRailBlockExtension.canMakeSlopes"
    )]
    fn rail_can_make_slopes();
}
