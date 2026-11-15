use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.getExplosionResistance`: the parts of `state` at `pos` may answer the
    /// block's resistance against `explosion`. The level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_explosion_resistance(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        explosion: &ExplosionStart<'_>,
        resistance: &mut Option<f32>,
    ) {
        self.fire_explosion_resistance(pos, state, explosion, resistance);
    }
}

impl Game<'_> {
    /// The resistance the parts of `state` at `pos` give it against `explosion`, in place of
    /// its static one, on a logical server. A negative or non-finite answer is refused and a
    /// panicking part contained, both with a warning: None keeps the static value.
    pub(in crate::game) fn part_explosion_resistance(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        explosion: &ExplosionStart<'_>,
    ) -> Option<f32> {
        if !self.is_logical_server()
            || !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::ExplosionResistance,
                state,
            )
        {
            return None;
        }
        let mut resistance = None;
        let pos = BlockPos::new(pos[0], pos[1], pos[2]);
        if let Err(error) = horizon_client::crash::contain(|| {
            self.with_level(|_, level, _| {
                level.native_explosion_resistance(pos, state, explosion, &mut resistance);
            });
        }) {
            log::warn!("explosion_resistance callback failed: {error}");
            return None;
        }
        match resistance {
            Some(value) if !(value.is_finite() && value >= 0.0) => {
                log::warn!("explosion_resistance refused {value}; using the block's own");
                None
            }
            answer => answer,
        }
    }
}
