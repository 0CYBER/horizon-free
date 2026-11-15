//! Opaque registration tokens; callers validate membership in their owning request table.
/// Cold authoring access to the opaque token representation.
pub fn block_key(slot: u32) -> crate::BlockKey {
    crate::BlockKey(slot)
}
/// Cold authoring access to the opaque token representation.
pub fn block_key_slot(key: crate::BlockKey) -> u32 {
    key.0
}

/// Validate a token using the current registration or installed lookup epoch.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn validate_block_key(slot: u32) -> Result<crate::BlockKey, &'static str> {
    crate::BlockKey::from_slot(slot)
}
