use super::*;

impl Game<'_> {
    /// Entity model layers built in code (NeoForge's
    /// `EntityRenderersEvent.RegisterLayerDefinitions`): push each one's id,
    /// `<mod id>:<model>#<layer>` as a `ModelLayerLocation` names it
    /// (`mymod:golem#main`), and its `LayerDefinition`, as the mod's
    /// `createBodyLayer()` builds it. `bake_layer` bakes one into the
    /// `ModelPart` tree `native_draw_model_part` draws. Asked once at start,
    /// on the client. An id in vanilla's namespace, not of that form, or
    /// registered already is logged and left out. API @API:model_layers@.
    pub(in crate::game) fn native_register_layer_definitions(
        context: &mod_api::RegistrationContext,
        layers: &mut Vec<(String, LayerDefinition)>,
    ) {
        Self::fire_register_layer_definitions(context, layers);
    }
}
