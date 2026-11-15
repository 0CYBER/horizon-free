use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.canDropFromExplosion`: the parts of `state` at `pos` may answer
    /// whether the block `explosion` destroys drops. The level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_can_drop_from_explosion(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        explosion: &ExplosionStart<'_>,
        drops: &mut Option<bool>,
    ) {
        self.fire_can_drop_from_explosion(pos, state, explosion, drops);
    }
}

impl Game<'_> {
    /// Whether the block `state` at `pos`, destroyed by `explosion`, drops, as its parts answer
    /// on a logical server. A panicking part is contained with a warning: None keeps vanilla's
    /// answer.
    pub(in crate::game) fn part_drops_from_explosion(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        explosion: &ExplosionStart<'_>,
    ) -> Option<bool> {
        if !self.is_logical_server()
            || !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::CanDropFromExplosion,
                state,
            )
        {
            return None;
        }
        let mut drops = None;
        let pos = BlockPos::new(pos[0], pos[1], pos[2]);
        if let Err(error) = horizon_client::crash::contain(|| {
            self.with_level(|_, level, _| {
                level.native_can_drop_from_explosion(pos, state, explosion, &mut drops);
            });
        }) {
            log::warn!("can_drop_from_explosion callback failed: {error}");
            return None;
        }
        drops
    }
}
