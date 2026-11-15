//! A block state turned as a structure template places it (vanilla's `BlockState.rotate`, and
//! NeoForge's `IBlockExtension.rotate(state, level, pos, rotation)`), which the parts of the
//! state's block may answer in `rotate`.
use crate::{BlockState, StructureRotation};

/// A structure template is about to place `state`, already mirrored, at `pos` turned by
/// `rotation`: `rotated` starts at the game's own turned state, and the parts of `state`'s block
/// may set another state of that block.
#[inline(never)]
pub fn native_rotate(
    context: &crate::OwningWorldgen<'_>,
    state: BlockState,
    pos: [i32; 3],
    rotation: StructureRotation,
    rotated: &mut BlockState,
) {
    crate::dispatch_rotate(context, state, pos, rotation, rotated);
}
