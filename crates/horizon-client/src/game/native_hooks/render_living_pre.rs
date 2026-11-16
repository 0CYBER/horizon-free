use super::*;

impl Game<'_> {
    /// Before a living entity is drawn (NeoForge's `RenderLivingEvent.Pre`): once a frame for each
    /// mob and player in view, before its pose, model and layers, with its render state and a draw
    /// posed at its feet, turned with its body. `canceled` skips all of the entity's drawing this
    /// frame and its `render_living_post`; uncanceled, `hidden` names the vanilla model's parts to
    /// leave out.
    pub(in crate::game) fn native_render_living_pre(
        &self,
        entity: &MobRender<'_>,
        draw: &mut BlockEntityDraw,
        hidden: &mut HiddenParts,
        canceled: &mut bool,
    ) {
        self.fire_render_living_pre(entity, draw, hidden, canceled);
    }
}
