use super::*;

impl Game<'_> {
    /// Positional `IBlockExtension.getAdjacentBlockPathType`: the neighbour's
    /// own classified `path_type` and position. `None` keeps fluid/vanilla fallback.
    pub(in crate::game) fn native_adjacent_block_path_type(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        path_type: mod_api::PathType,
        adjacent: &mut Option<mod_api::PathType>,
    ) {
        self.fire_adjacent_block_path_type(pos, state, path_type, adjacent);
    }
}
