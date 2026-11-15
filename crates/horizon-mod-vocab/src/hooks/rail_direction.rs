//! The `rail_direction` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The track the rail `state` at `pos` lays for `cart`
    /// (`IBaseRailBlockExtension.getRailDirection`), so a switch track or a one-way rail can
    /// turn a cart differently from its stored shape. `shape` starts at the rail's `shape`
    /// property. `cart` is the minecart moving on it, or `None` when the rail asks for itself:
    /// as it connects to its neighbours (`RailState`) and as a minecart item is placed on it.
    /// The cart's movement asks wherever it reads the track under it. Asked on both sides,
    /// since a client predicts its own cart's motion: register the mod on both and answer the
    /// same on each, from the level host and the event alone. With no part attached to the
    /// state, the cost is one table lookup per rail read. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "collect",
        out = "shape",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::rails::GameLevel<'_>::native_rail_direction",
        neoforge = "IBaseRailBlockExtension.getRailDirection"
    )]
    fn rail_direction();
}
