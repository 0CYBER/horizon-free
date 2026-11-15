//! Opaque registration tokens; callers validate membership in their owning request table.
/// Cold authoring access to the opaque token representation.
pub fn effect_key(slot: u32) -> crate::EffectKey {
    crate::EffectKey(slot)
}
/// Cold authoring access to the opaque token representation.
pub fn effect_key_slot(key: crate::EffectKey) -> u32 {
    key.0
}

/// Validate a token using the current registration or installed lookup epoch.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn validate_effect_key(slot: u32) -> Result<crate::EffectKey, &'static str> {
    crate::EffectKey::from_slot(slot)
}
