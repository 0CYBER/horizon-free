use super::*;

impl Game<'_> {
    /// `IBlockExtension.getStateAtViewpoint`: the state the camera at `viewpoint` sees of
    /// `state` at `pos`. `seen` starts at `state`; the block's parts may change it.
    pub(in crate::game) fn native_state_at_viewpoint(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        viewpoint: [f64; 3],
        seen: &mut BlockStateId,
    ) {
        self.fire_state_at_viewpoint(state, pos, viewpoint, seen);
    }
}
