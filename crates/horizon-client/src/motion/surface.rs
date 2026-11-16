//! What the block an entity stands on or in gives it, as the block's parts answer: its friction
//! (`IBlockExtension.getFriction`), its jump and speed factors, and whether it is a ladder.
use crate::{blocks::BlockFacts, world::World};
use horizon_core::position::BlockPos;
use horizon_world::{
    movement_properties::BlockFactorEdits, native_load_context::LoopContext, registry::BlockStateId,
};

/// A block state's friction (`IBlockExtension.getFriction`) is being read for an entity on it.
/// `friction` starts at the block's own value and the block's parts may change it; an answer that
/// is not within `0..=2` (NaN included) is dropped for the block's own value. Fires on replayed
/// ticks too, so a listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_friction(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    pos: BlockPos,
    state: BlockStateId,
    friction: &mut f32,
) {
    let own = *friction;
    crate::native_client_hooks::friction(context, entity, kind, pos, state, friction);
    if !(0.0..=2.0).contains(&*friction) {
        *friction = own;
    }
}

/// The friction of the block at `pos` for the entity `entity` of `kind` on it: the block's own
/// `StateFacts` value, through [`native_friction`].
pub fn block_friction(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    world: &World,
    facts: &BlockFacts,
    pos: BlockPos,
) -> f32 {
    let state = world.block(pos.x, pos.y, pos.z);
    let mut friction = facts.get(state).friction;
    native_friction(context, entity, kind, pos, state, &mut friction);
    friction
}

/// A block state's jump factor (`Block.getJumpFactor`) is being read (beyond NeoForge: no event).
/// Its parts may change it. Fires on replayed ticks too, so a listener must be a pure function of
/// its inputs.
#[inline(never)]
pub fn native_block_jump_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    crate::native_client_hooks::block_jump_factor(context, state, factor);
}

/// A block state's speed factor (`Block.getSpeedFactor`) is being read (beyond NeoForge: no
/// event). Its parts may change it. Fires on replayed ticks too, so a listener must be a pure
/// function of its inputs.
#[inline(never)]
pub fn native_block_speed_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    crate::native_client_hooks::block_speed_factor(context, state, factor);
}

/// Whether a block is a ladder to a living entity (`IBlockExtension.isLadder`, which
/// `LivingEntity.onClimbable` asks of the block the entity stands in) is being decided. `ladder`
/// starts with the `#minecraft:climbable` answer and the block's parts may change it. Fires on
/// replayed ticks too, so a listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_is_ladder(
    context: &LoopContext,
    entity: u32,
    pos: BlockPos,
    state: BlockStateId,
    ladder: &mut bool,
) {
    crate::native_client_hooks::is_ladder(context, entity, pos, state, ladder);
}

/// The factors the `block_jump_factor` and `block_speed_factor` parts give the blocks an entity
/// reads, for the loop of the side that ticks it.
pub(crate) struct ModBlockFactors<'a>(pub &'a LoopContext);
impl BlockFactorEdits for ModBlockFactors<'_> {
    fn jump(&self, state: BlockStateId, mut factor: f32) -> f32 {
        native_block_jump_factor(self.0, state, &mut factor);
        factor
    }
    fn speed(&self, state: BlockStateId, mut factor: f32) -> f32 {
        native_block_speed_factor(self.0, state, &mut factor);
        factor
    }
}
