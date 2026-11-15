//! Opaque registration tokens; callers validate membership in their owning request table.
/// Cold authoring access to the opaque token representation.
pub fn block_entity_type_key(slot: u32) -> crate::BlockEntityTypeKey {
    crate::BlockEntityTypeKey(slot)
}
/// Cold authoring access to the opaque token representation.
pub fn block_entity_type_key_slot(key: crate::BlockEntityTypeKey) -> u32 {
    key.0
}

/// Validate a token using the current registration or installed lookup epoch.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn validate_block_entity_type_key(
    slot: u32,
) -> Result<crate::BlockEntityTypeKey, &'static str> {
    crate::BlockEntityTypeKey::from_slot(slot)
}
