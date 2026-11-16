pub use client::particle_keys::ParticleKey;
mod __private {
    pub use crate::client::particle_keys::{
        particle_key, particle_kind, particle_name, particle_named_key, particle_vanilla_key,
        particles_resolved, resolve_particles,
    };
    pub use horizon_mod_runtime::registry_keys::ParticleKind;
}
