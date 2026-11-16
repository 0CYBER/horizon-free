//! The `state_for_placement` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0089

pub trait Hooks {
    /// A block is about to take the state it is placed in
    /// (`Block.getStateForPlacement(BlockPlaceContext)`), `state` the block's
    /// own default state. Answer only for your blocks: put the state you want
    /// in `answer` as `Some(Some(state))` — a state of that block, as vanilla
    /// sets properties on its own `defaultBlockState` (another block's is
    /// ignored) — or refuse the placement with `Some(None)`: the block goes
    /// nowhere and the stack is kept, as vanilla's `null`. Left empty, the
    /// block's own placement runs. `context` is
    /// vanilla's `BlockPlaceContext` (the clicked face, the placer's rotation,
    /// the clicked position, whether the clicked block is being replaced, the
    /// item in hand) and `fluid` the fluid in the position the block would go
    /// in, which a waterlogged block reads. Build the state from `state` with
    /// `native_state_with_property`, and know whether `state` is yours with
    /// `native_state_block`. It is asked for every placer that goes
    /// through vanilla's placement (a player's `BlockItem.place`, a
    /// dispenser's `DirectionalPlaceContext`), and the answer still takes its
    /// survival and entity obstruction checks. A trait mod returns
    /// `Some(Some(state))` to choose a state, `Some(None)` to refuse, or
    /// `None` to leave the answer to the next mod or vanilla. Changes to the event's fluid
    /// copy are mod-provided data forwarded to later Parts, never consumed by the engine.
    /// Authoritative servers only.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_state_for_placement",
        neoforge = "none (vanilla Block.getStateForPlacement; NeoForge adds no event or extension for it)"
    )]
    fn state_for_placement();
}
