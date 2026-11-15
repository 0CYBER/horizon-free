use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The HUD (not drawn under F1), after the minimap (`RenderGuiEvent.Post`):
    /// place a box with `corners.place` (boxes stack away from their edge) and
    /// draw it with `native_fill` and `native_text`. Skip a corner
    /// `native_corner_shown` says is taken.
    pub(in crate::game) fn native_render_gui_post(
        &self,
        draw: &mut HudBatch,
        corners: &mut Corners,
    ) {
        self.fire_render_gui_post(draw, corners);
    }
}
