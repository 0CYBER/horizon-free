use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before one of the first-person view's block overlays (`ScreenEffectRenderer`) is drawn,
    /// once a frame each is shown: `Block` with the view-blocking `state` at `pos` the camera is
    /// inside, `Water` with the `state` at the eye's block `pos` while the eye is under water, and
    /// `Fire` with fire's default `state` at the player's block `pos` while they burn. Never for a
    /// spectator; `Fire` only in first person. Set `canceled` to leave that overlay out; the
    /// frame, the other block overlays and the HUD's camera overlays are drawn as before. Set
    /// `custom_draw` to draw it yourself (`block_screen_effect_draw`), canceled or not.
    /// `partial_tick` is the world's.
    /// Client-side.
    pub(in crate::game) fn native_extract_block_screen_effect(
        &self,
        effect: mod_api::BlockScreenEffect,
        state: BlockStateId,
        pos: BlockPos,
        partial_tick: f32,
        canceled: &mut bool,
        custom_draw: &mut bool,
    ) {
        self.fire_extract_block_screen_effect(
            effect,
            state,
            pos,
            partial_tick,
            canceled,
            custom_draw,
        );
    }
}
