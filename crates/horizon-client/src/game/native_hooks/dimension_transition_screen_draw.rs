use super::*;

impl Game<'_> {
    /// Draw the registered presentation, leaving the load tracker in charge of closing it.
    pub(in crate::game) fn native_dimension_transition_screen_draw(
        &self,
        screen: &str,
        reason: mod_api::DimensionTransitionReason,
        progress: Option<f32>,
        draw: &mut HudBatch,
    ) {
        self.fire_dimension_transition_screen_draw(screen, reason, progress, draw);
    }
}
