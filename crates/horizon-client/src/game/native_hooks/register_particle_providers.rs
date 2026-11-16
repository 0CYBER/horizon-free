use super::*;

impl Game<'_> {
    /// The code-defined particle providers of a mod's own particle types (NeoForge's
    /// `RegisterParticleProvidersEvent`): `register(name, provider)`, and every particle of the
    /// type `name` (`ParticleOptions::named("mymod:spark")`) is moved each tick by
    /// `ParticleProvider::tick`. The sprite set and the declarative spawn values stay in
    /// `assets/<namespace>/particles/<path>.json`. A later registration for a name replaces the
    /// earlier one; a name no sprite set matches is logged and draws nothing. Asked once at start
    /// on the client, before resources load; the game resolves each type's provider when the
    /// sprite sets load, on every resource load as well.
    pub(in crate::game) fn native_register_particle_providers(
        context: &mod_api::RegistrationContext,
        providers: &mut dyn ParticleProviders,
    ) {
        Self::fire_register_particle_providers(context, providers);
    }
}
