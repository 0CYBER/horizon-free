use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    pub(in crate::game) fn finish_render_frame(&mut self, elapsed: Duration) {
        let (world_partial_tick, partial_tick) = self
            .extras
            .frame_partial_ticks
            .take()
            .unwrap_or_else(|| self.frame_partial_ticks());
        self.native_render_frame_post(elapsed, world_partial_tick, partial_tick);
    }

    /// Once a frame, in a world or not, right after the frame was rendered
    /// (`RenderFrameEvent.Post`).
    pub(in crate::game) fn native_render_frame_post(
        &mut self,
        elapsed: Duration,
        world_partial_tick: f32,
        partial_tick: f32,
    ) {
        self.fire_render_frame_post(elapsed, world_partial_tick, partial_tick);
    }
}
