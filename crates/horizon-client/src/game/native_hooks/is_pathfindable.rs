use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `BlockBehaviour.isPathfindable`: set `pathfindable` to change whether a path of `kind`
    /// may go through `state`. It starts with the block's own answer and depends on the state
    /// and the type alone, so the game asks once per state when the parts resolve (at load and on
    /// every `/reload`) and path finding and spawning read the answers. Server-side.
    pub(in crate::game) fn native_is_pathfindable(
        &self,
        state: BlockStateId,
        kind: PathComputationType,
        pathfindable: &mut bool,
    ) {
        self.fire_is_pathfindable(state, kind, pathfindable);
    }
}
