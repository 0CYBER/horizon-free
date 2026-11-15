use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Draw one active debug renderer (`native_register_debug_renderers`), asked for each
    /// view of a world. A renderer that is not active is never asked.
    pub(in crate::game) fn native_debug_renderer_draw(
        &self,
        id: &str,
        view: &mod_api::WorldRenderView,
        draw: &mut horizon_client::world_geometry::WorldGeometry,
    ) {
        self.fire_debug_renderer_draw(id, view, draw);
    }
}
