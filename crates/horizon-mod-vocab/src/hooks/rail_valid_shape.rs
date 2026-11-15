//! The `rail_valid_shape` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the rail `state` may take `shape` (`IBaseRailBlockExtension.isValidRailShape`),
    /// asked as it connects to its neighbours (`RailState`) before the shape is written.
    /// `valid` starts true when the rail's `shape` property has that value; set false to refuse
    /// it, and the rail keeps its current shape. A part can't make valid a shape the property
    /// lacks. Asked on an authoritative server: answer from the state and the shape, without
    /// touching the world. With no part attached to the state, the cost is one table lookup.
    /// Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "valid",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::rails::GameLevel<'_>::native_rail_valid_shape",
        neoforge = "IBaseRailBlockExtension.isValidRailShape"
    )]
    fn rail_valid_shape();
}
