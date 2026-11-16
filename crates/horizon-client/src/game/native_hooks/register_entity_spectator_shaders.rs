use super::*;

impl Game<'_> {
    /// Collect typed client spectator post-effect registrations once at startup.
    pub(in crate::game) fn native_register_entity_spectator_shaders(
        context: &mod_api::RegistrationContext,
        shaders: &mut Vec<mod_api::EntitySpectatorShader>,
    ) {
        Self::fire_register_entity_spectator_shaders(context, shaders);
    }
}
