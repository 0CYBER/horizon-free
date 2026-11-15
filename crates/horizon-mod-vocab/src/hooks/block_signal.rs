//! The `block_signal` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0054

pub trait Hooks {
    /// The redstone signal the block `state` at `pos` gives on its `face`, the
    /// side that touches the block reading it: `BlockState.getDirectSignal`
    /// when `direct` (what passes through a block it touches, as a lever's
    /// does), else `getSignal`. `signal` holds vanilla's answer: change it when
    /// the block is yours (0 to 15; anything else is clamped). Asked very often,
    /// from inside the level's update, so answer from your own state: check the
    /// block first, and don't read or change the world here. When the answer
    /// changes, and once the block is placed, call
    /// `level.update_neighbors(host, pos, state, NeighbourUpdate::Neighbours)`. Authoritative
    /// servers only.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_signal",
        neoforge = "none (vanilla BlockBehaviour.getSignal and getDirectSignal; NeoForge adds no event or extension for it)"
    )]
    fn block_signal();
}
