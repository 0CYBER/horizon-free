use super::*;

impl Game<'_> {
    /// After a living entity is drawn (NeoForge's `RenderLivingEvent.Post`): once a frame for each
    /// mob and player in view whose pre hook wasn't canceled, after its model and its layers, with
    /// its render state, a humanoid model's posed parts and a draw posed at its feet.
    pub(in crate::game) fn native_render_living_post(
        &self,
        entity: &MobRender<'_>,
        parts: Option<&PosedParts>,
        draw: &mut BlockEntityDraw,
    ) {
        self.fire_render_living_post(entity, parts, draw);
    }
}
