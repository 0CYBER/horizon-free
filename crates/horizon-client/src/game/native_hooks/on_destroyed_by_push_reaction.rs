use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.onDestroyedByPushReaction`: a piston's move destroys the block `state`
    /// at `pos`, `direction` the way the blocks move, before the cell becomes air, `fluid` the
    /// fluid state the cell holds. Writes to the cells of the move are overwritten; `ticks` is
    /// the move's.
    #[inline(always)]
    pub(in crate::game) fn native_on_destroyed_by_push_reaction(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        fluid: mod_api::FluidState,
    ) {
        self.fire_on_destroyed_by_push_reaction(ticks, dimension, pos, state, direction, fluid);
    }

    /// `LevelAccess::on_destroyed_by_push_reaction`: on an authoritative server, the parts of
    /// `state` hear it. A panicking part is contained with a warning, and the move goes on.
    pub(in crate::game) fn destroyed_by_push_reaction(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        fluid: horizon_world::fluid::FluidState,
    ) {
        if !self.native_authoritative
            || !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::OnDestroyedByPushReaction,
                state,
            )
        {
            return;
        }
        let Some(fluid) = crate::game::native_world_read::api_fluid_state(fluid) else {
            return;
        };
        let dimension = self.dimension;
        if let Err(error) = horizon_client::crash::contain(|| {
            self.native_on_destroyed_by_push_reaction(
                ticks, dimension, pos, state, direction, fluid,
            );
        }) {
            log::warn!("on_destroyed_by_push_reaction callback failed: {error}");
        }
    }
}
