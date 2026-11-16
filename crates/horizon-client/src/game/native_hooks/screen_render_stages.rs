use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before the open screen draws itself (`ScreenEvent.Render.Pre`): draw
    /// here to put something under it, or set `canceled` and the screen is not
    /// drawn (nor its Background or Foreground phase).
    pub(in crate::game) fn native_screen_render_pre(
        &self,
        draw: &mut HudBatch,
        screen: &ScreenView<'_>,
        canceled: &mut bool,
    ) {
        self.fire_screen_render_pre(draw, screen, canceled);
    }
    /// After the panel, before contents (`ScreenEvent.Render.Background`).
    pub(in crate::game) fn native_screen_render_background(
        &self,
        draw: &mut HudBatch,
        screen: &ScreenView<'_>,
    ) {
        self.fire_screen_render_background(draw, screen);
    }
    /// After contents, before tooltips/carried stack (`ScreenEvent.Render.Foreground`).
    pub(in crate::game) fn native_screen_render_foreground(
        &self,
        draw: &mut HudBatch,
        screen: &ScreenView<'_>,
    ) {
        self.fire_screen_render_foreground(draw, screen);
    }
}
