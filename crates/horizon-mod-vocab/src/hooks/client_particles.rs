//! Upstream prose, retained with fork routing described in the modding book.
pub trait Notes {
    #[notes = "`Host::add_particle(&ParticleOptions, position, velocity) -> bool` submits one particle to this client's loaded world. It preserves all option and velocity slots, including encoded color/size arguments, and uses the existing built-in and named-sprite providers with normal camera-distance, Particles-setting and population limits. True means submitted, not necessarily visible; missing providers/sprite sets draw nothing. It returns false on servers, without a client player/level, or for non-finite numeric input. A mod's own particle type gets code through `register_particle_providers`; new trigger hooks remain separate. Bundles need \"hooks\": @API:client_particles@."]
    fn client_particles();
}
