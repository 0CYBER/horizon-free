use super::*;

impl Game<'_> {
    /// The render groups a mod's particles draw in (NeoForge's `RegisterParticleGroupsEvent`):
    /// push a `ParticleGroup` per group, its id `<mod id>:<path>`, how it blends and the particle
    /// types in it. Asked once at start on the client, before resources load; the game resolves
    /// each type's group when the sprite sets load, on every resource load as well.
    pub(in crate::game) fn native_register_particle_groups(
        context: &mod_api::RegistrationContext,
        groups: &mut Vec<ParticleGroup>,
    ) {
        Self::fire_register_particle_groups(context, groups);
    }
}
