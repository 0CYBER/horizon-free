use super::*;

impl GameLevel<'_> {
    /// A crop-like block at `pos` is about to grow on a random tick
    /// (NeoForge's `CropGrowEvent.Pre`): crops, stems, nether wart, cocoa,
    /// sweet berries and bamboo at their own position; cactus and chorus
    /// flowers at the block above; sugar cane at its own position; kelp,
    /// weeping and twisting vines and cave vines at the block they grow
    /// into. `state` is the growing block. Leave `result` at `Default` for
    /// vanilla's own chance, or set `True` to grow now or `False` to stop
    /// it (`DO_NOT_GROW`). Vanilla's random roll is drawn either way, in
    /// vanilla's order. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_crop_grow_pre(
        &mut self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        result: &mut TriState,
    ) {
        self.fire_crop_grow_pre(dimension, pos, state, result);
    }

    /// A crop-like block grew (NeoForge's `CropGrowEvent.Post`).
    /// `original_state` is the block before it grew and `state` what is at
    /// `pos` now. Sugar cane reports the new cane above it, and kelp and
    /// vines the new head, as both states. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_crop_grow_post(
        &mut self,
        dimension: LevelDimension,
        pos: BlockPos,
        original_state: BlockStateId,
        state: BlockStateId,
    ) {
        self.fire_crop_grow_post(dimension, pos, original_state, state);
    }
}
