use super::*;

impl Game<'_> {
    /// Positional `IBlockExtension.getBlockPathType`, before all vanilla classification.
    /// `None` declines; the readonly host reads this same owning level.
    pub(in crate::game) fn native_block_path_type(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        path_type: &mut Option<mod_api::PathType>,
    ) {
        self.fire_block_path_type(pos, state, path_type);
    }
}
