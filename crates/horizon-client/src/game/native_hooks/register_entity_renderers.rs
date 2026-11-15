use super::*;

impl Game<'_> {
    /// The code-defined renderers of entity types (NeoForge's
    /// `EntityRenderersEvent.RegisterRenderers`, `registerEntityRenderer`): `register(type,
    /// renderer)`, and every entity of the type `type` (`mymod:bolt`) in view is drawn by
    /// `EntityRenderer::submit` in place of the type's own drawing. A later registration for a
    /// type replaces the earlier one; a type the game does not know is logged with its id and
    /// draws nothing new. Asked once at start on the client, before resources load; the game
    /// resolves each type's renderer on every resource load.
    pub(in crate::game) fn native_register_entity_renderers(
        context: &mod_api::RegistrationContext,
        renderers: &mut dyn EntityRenderers,
    ) {
        Self::fire_register_entity_renderers(context, renderers);
    }
}
