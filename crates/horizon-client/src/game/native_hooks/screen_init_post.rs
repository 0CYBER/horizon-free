use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A screen was observed opening or changing rectangle, at most once per frame
    /// (`ScreenEvent.Init.Post`).
    pub(in crate::game) fn native_screen_init_post(&self, screen: &ScreenView<'_>) {
        self.fire_screen_init_post(screen);
    }
}
