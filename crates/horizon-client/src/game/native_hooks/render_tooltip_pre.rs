use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `RenderTooltipEvent.Pre`: a tooltip about to be placed and drawn. `place` is the point
    /// it hangs from, before the positioner's offset and clamp; change it to move the tooltip.
    /// `elements` are its lines then its images, read-only; `stack` is the item it describes.
    /// Set `cancel` and nothing is drawn, and no `render_tooltip_texture` is asked. Client-side.
    pub(in crate::game) fn native_render_tooltip_pre(
        &self,
        call: mod_api::__private::RenderTooltipPreCall<'_>,
    ) {
        self.fire_render_tooltip_pre(call);
    }
}
