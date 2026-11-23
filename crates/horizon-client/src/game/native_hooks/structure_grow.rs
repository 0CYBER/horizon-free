use super::*;

impl GameLevel<'_> {
    /// A tree, mushroom or fungus grew from `source` and placed `placed`, the blocks it changed,
    /// in order. The blocks are already in the world. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_structure_grow(
        &self,
        dimension: LevelDimension,
        source: BlockPos,
        placed: &[[i32; 3]],
    ) {
        self.fire_structure_grow(dimension, source, placed);
    }
}
