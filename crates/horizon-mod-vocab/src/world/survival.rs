//! Blocks' survival as they are placed (vanilla's `BlockState.canSurvive`) and as a plant
//! picks the ground it stands on (`VegetationBlock.mayPlaceOn`), which the parts of the
//! state's block may answer in `can_survive_v2`, `may_place_on_v2` and `can_sustain_plant_v2`.
//!
//! These are the worldgen halves of the three published survival hooks: asked on the worldgen
//! worker threads while a feature places a block, and on the server thread with the same
//! tables, so a feature and a hand placement get one answer. There is no `Host` and nothing to
//! write: what a part may read is the event's own arguments (the state, the position, and for a
//! plant its soil).
use crate::{BlockPos, BlockState};

/// `BlockState.canSurvive` is about to be asked for `state` at `pos` while a feature places it:
/// the parts of `state`'s block may answer `survives`, which replaces the block's own rule.
#[inline(never)]
pub fn native_can_survive_v2(
    context: &crate::OwningWorldgen<'_>,
    pos: BlockPos,
    state: BlockState,
    survives: &mut Option<bool>,
) {
    crate::dispatch_can_survive_v2(context, pos, state, survives);
}

/// `VegetationBlock.mayPlaceOn` is about to be asked for the plant `state` on `ground` at
/// `ground_pos` while a feature places it: the parts of `state`'s block may answer `may`, which
/// replaces the plant's own rule.
#[inline(never)]
pub fn native_may_place_on_v2(
    context: &crate::OwningWorldgen<'_>,
    state: BlockState,
    ground: BlockState,
    ground_pos: BlockPos,
    may: &mut Option<bool>,
) {
    crate::dispatch_may_place_on_v2(context, state, ground, ground_pos, may);
}

/// `IBlockExtension.canSustainPlant` is about to be asked for the soil `ground` at `ground_pos`
/// and the plant `state` above it while a feature places the plant: the parts of `ground`'s
/// block may answer `sustains`, which replaces the plant's own `mayPlaceOn`.
#[inline(never)]
pub fn native_can_sustain_plant_v2(
    context: &crate::OwningWorldgen<'_>,
    state: BlockState,
    ground: BlockState,
    ground_pos: BlockPos,
    sustains: &mut Option<bool>,
) {
    crate::dispatch_can_sustain_plant_v2(context, state, ground, ground_pos, sustains);
}
