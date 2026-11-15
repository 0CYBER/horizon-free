use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Draw a block overlay a mod said it draws itself (`custom_draw` in
    /// `extract_block_screen_effect`), before vanilla's overlay of that kind, once a frame each.
    /// A canceled overlay is asked too. Set `replace_vanilla` to leave vanilla's overlay out.
    pub(in crate::game) fn native_block_screen_effect_draw(
        &self,
        call: super::fire::BlockScreenEffectDrawCall<'_>,
    ) {
        self.fire_block_screen_effect_draw(call);
    }
}
