//! The `affect_neighbors_after_removal` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Block.affectNeighborsAfterRemoval` for a mod's block: an authoritative server removed the
    /// block `state` from `pos` (`moved`: a piston pushed it) and has run its template class's own
    /// updates. Push onto `updates` the neighbour updates it wants, in order: `OutputSignal`
    /// (`updateNeighbourForOutputSignal`), `Neighbours` (`updateNeighborsAt`) and
    /// `NeighboursExcept(direction)` (`updateNeighborsAtExceptFromFacing`); the level runs each
    /// at once, as the block's own override would. Answered from inside the level's write: it
    /// reads, and cannot write blocks or spawn entities.
    #[hook(
        side = "server",
        contract = "collect",
        out = "updates",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::affect_neighbors_after_removal::GameLevel<'_>::native_affect_neighbors_after_removal",
        neoforge = "none (vanilla Block.affectNeighborsAfterRemoval; NeoForge adds no event or extension for it)"
    )]
    fn affect_neighbors_after_removal();
}
