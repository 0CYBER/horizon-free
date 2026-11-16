use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// One stage of a world view (NeoForge's `RenderLevelStageEvent`), asked for each stage in
    /// frame order as the view is extracted; what `draw` takes is drawn at `stage`.
    pub(in crate::game) fn native_render_level_stage(
        &self,
        stage: mod_api::RenderLevelStage,
        view: &mod_api::WorldRenderView,
        draw: &mut horizon_client::world_geometry::WorldGeometry,
    ) {
        self.fire_render_level_stage(stage, view, draw);
    }
}
