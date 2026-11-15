use super::*;

impl Game<'_> {
    /// The layers a mod adds to living entity renderers (NeoForge's
    /// `EntityRenderersEvent.AddLayers`): `add(target, layer)`, and every living entity of the
    /// type, or every player of the player model, in view draws `EntityRenderLayer::submit` after
    /// its model and the layers added before. `entity_types`, `is_living`, `player_models` and
    /// `has_mod_renderer` tell what renderers this load built. A target that is no living entity
    /// type is refused by name. Asked on the client on every resource load, the first one and each
    /// reload, after the renderers resolve (`register_entity_renderers`): layers don't carry over
    /// from one load to the next, so add them again each time.
    pub(in crate::game) fn native_register_entity_layers(
        context: &mod_api::RegistrationContext,
        layers: &mut dyn EntityLayers,
    ) {
        Self::fire_register_entity_layers(context, layers);
    }
}
