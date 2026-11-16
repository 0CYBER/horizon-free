//! The `analog_output` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0073

pub trait Hooks {
    /// `BlockState.getAnalogOutputSignal`: the signal a comparator reads from
    /// the block `state` at `pos`, `face` the side that touches the comparator
    /// (or the conductor in front of it). `output` holds vanilla's answer:
    /// change it when the block is yours (0 to 15; anything else is clamped).
    /// Asked only for blocks `has_analog_output` answered for, from
    /// inside the level's update: don't change the world here. When the
    /// answer changes without the block's state changing, call
    /// `level.update_neighbors(host, pos, state, NeighbourUpdate::OutputSignal)`; a change of
    /// state through
    /// `native_set_block` or `native_set_block_property` (neighbour updates
    /// on) tells the comparators itself, as vanilla's `Level.setBlock` does.
    /// Authoritative servers only.
    #[hook(
        side = "server",
        contract = "collect",
        out = "output",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_analog_output",
        neoforge = "none (vanilla BlockBehaviour.getAnalogOutputSignal; NeoForge adds no event or extension for it)"
    )]
    fn analog_output();
}
