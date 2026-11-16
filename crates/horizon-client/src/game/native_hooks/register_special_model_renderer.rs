use super::*;

impl Game<'_> {
    /// The special item model types a mod draws (NeoForge's `RegisterSpecialModelRendererEvent`):
    /// `register(type, renderer)`, and every `minecraft:special` item model whose `model` object
    /// has the type `type` (`mymod:skillet`) is drawn by `SpecialModelRenderer::extract`. A later
    /// registration of a type replaces the earlier one. Asked once at start on the client, before
    /// resources load.
    pub(in crate::game) fn native_register_special_model_renderer(
        context: &mod_api::RegistrationContext,
        renderers: &mut dyn SpecialModelRenderers,
    ) {
        Self::fire_register_special_model_renderer(context, renderers);
    }
}
