/// Load-owned particle key requests, resolved before any world can emit particles.
pub trait ParticlesRegistrar {
    /// Request a vanilla particle or a custom sprite provider by canonical registry ID.
    /// Unknown IDs are reported with the requesting mod when the content registry loads.
    fn particle_key(&mut self, id: &str) -> crate::ParticleKey;
}
