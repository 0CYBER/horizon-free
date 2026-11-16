//! Opaque registration tokens; callers validate membership in their owning request table.
/// Cold authoring access to the opaque token representation.
pub fn particle_key(slot: u32) -> crate::ParticleKey {
    crate::ParticleKey(slot)
}
/// Cold authoring access to the opaque token representation.
pub fn particle_key_slot(key: crate::ParticleKey) -> u32 {
    key.0
}

/// Validate a token using the current registration or installed lookup epoch.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn validate_particle_key(slot: u32) -> Result<crate::ParticleKey, &'static str> {
    crate::ParticleKey::from_slot(slot)
}
