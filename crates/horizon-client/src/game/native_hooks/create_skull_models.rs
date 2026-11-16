use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Skull models for a mod's own skull types (NeoForge's
    /// `EntityRenderersEvent.CreateSkullModels`): push a `SkullModel` per type. Asked once at
    /// start, on the client. API @API:create_skull_models@.
    pub(in crate::game) fn native_create_skull_models(
        context: &mod_api::RegistrationContext,
        models: &mut Vec<SkullModel>,
    ) {
        Self::fire_create_skull_models(context, models);
    }
}
