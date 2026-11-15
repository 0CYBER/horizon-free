//! Opaque registration tokens; callers validate membership in their owning request table.
/// Cold authoring access to the opaque token representation.
pub fn fluid_key(slot: u32) -> crate::FluidKey {
    crate::FluidKey(slot)
}
/// Cold authoring access to the opaque token representation.
pub fn fluid_key_slot(key: crate::FluidKey) -> u32 {
    key.0
}

/// Validate a token using the current registration or installed lookup epoch.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn validate_fluid_key(slot: u32) -> Result<crate::FluidKey, &'static str> {
    crate::FluidKey::from_slot(slot)
}
