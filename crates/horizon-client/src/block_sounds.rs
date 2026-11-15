//! A block state's sound type as its parts answer (`IBlockExtension.getSoundType`): what the game
//! reads to play a block's step, fall, place or break sound, or a sniffer's dig.
use crate::blocks::{BlockFacts, Sounds};
use crate::mobs::{Mob, MobWorld};
use horizon_core::position::BlockPos;
use horizon_world::{native_load_context::LoopContext, registry::BlockStateId};
use mod_api::{BlockSoundCause, BlockSoundType};
use std::sync::Arc;

/// The sound type of a block state (`IBlockExtension.getSoundType`) is being read to play the
/// sound `cause` names. `sound` starts as the state's own and the block's parts may change it; an
/// answer whose volume or pitch is not finite and at least 0, or with an event that is not a
/// `namespace:path` id, is dropped whole for the state's own. `entity` is the stepping, landing,
/// placing or digging entity's id and type, `None` for a break or a placement by no entity. Fires
/// on the side that plays the sound, so a listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_sound_type(
    context: &LoopContext,
    pos: BlockPos,
    state: BlockStateId,
    entity: Option<(u32, &str)>,
    cause: BlockSoundCause,
    sound: &mut BlockSoundType,
) {
    crate::native_client_hooks::sound_type(context, pos, state, entity, cause, sound);
}

/// `sounds`, a state's own sound type (`BlockState.getSoundType()`), as the mod API's record.
pub fn own(sounds: &Sounds) -> BlockSoundType {
    BlockSoundType {
        volume: sounds.volume,
        pitch: sounds.pitch,
        break_sound: Arc::clone(&sounds.break_sound),
        step: Arc::clone(&sounds.step),
        place: Arc::clone(&sounds.place),
        hit: Arc::clone(&sounds.hit),
        fall: Arc::clone(&sounds.fall),
    }
}

/// The sound type of `state` at `pos` for `cause`: the state's own (`facts`), through
/// [`native_sound_type`].
pub fn sound_type(
    context: &LoopContext,
    facts: &BlockFacts,
    pos: BlockPos,
    state: BlockStateId,
    entity: Option<(u32, &str)>,
    cause: BlockSoundCause,
) -> BlockSoundType {
    let mut sound = own(&facts.get(state).sounds);
    native_sound_type(context, pos, state, entity, cause, &mut sound);
    sound
}

impl MobWorld<'_> {
    /// The sound type of `state` at `pos` that the mob `m` steps or lands on (`cause`), as the
    /// block's parts answer.
    pub(crate) fn block_sound_type(
        &self,
        m: &Mob,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
        cause: BlockSoundCause,
    ) -> BlockSoundType {
        let (pos, entity) = (BlockPos::new(x, y, z), Some((m.id, m.kind.id.as_str())));
        sound_type(self.loop_context, self.facts, pos, state, entity, cause)
    }
}
