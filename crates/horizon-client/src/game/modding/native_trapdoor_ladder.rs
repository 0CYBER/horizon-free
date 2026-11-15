// game-module-visibility: pub(crate)
//! The executable's side of `makes_open_trapdoor_above_climbable`, which the client library asks
//! through its callbacks (`native_client_hooks`): the parts of the block under an open trapdoor
//! decide whether the trapdoor climbs like a ladder.
use super::api_values::{TryIntoApi, block_state};

/// Ask the parts of `state`, the block at `pos` under the open trapdoor `trapdoor`, whether the
/// trapdoor climbs. With no part on `state`, nothing is converted and `climbable` keeps its answer.
pub(crate) fn makes_open_trapdoor_above_climbable(
    context: &horizon_world::native_load_context::LoopContext,
    state: horizon_world::registry::BlockStateId,
    pos: horizon_core::position::BlockPos,
    trapdoor: horizon_world::registry::BlockStateId,
    climbable: &mut bool,
) {
    if !block_state::block_listens(mod_api::HookId::MakesOpenTrapdoorAboveClimbable, state) {
        return;
    }
    let (Ok(state), Ok(trapdoor)) = (state.try_into_api(), trapdoor.try_into_api()) else {
        return;
    };
    mod_api::dispatch_makes_open_trapdoor_above_climbable(
        &horizon_client_render::api_context::LoopReader(context).context(),
        state,
        pos,
        trapdoor,
        climbable,
    );
}
