use super::*;

impl Game<'_> {
    /// The models of your world fluids (NeoForge's `RegisterFluidModelsEvent`): push a
    /// `mod_api::FluidModel` per fluid whose look doesn't say all of it. Its
    /// sprites are block atlas textures, baked at every resource load from that pack's files; the
    /// tint stays the look's. A fluid without one draws its look's sprite on every surface, or
    /// water's textures when it has none. Asked at startup and on resource reload before baking, and
    /// only on the client.
    pub(in crate::game) fn native_register_fluid_models(
        context: &mod_api::RegistrationContext,
        models: &mut Vec<mod_api::FluidModel>,
    ) {
        Self::fire_register_fluid_models(context, models);
    }
}
