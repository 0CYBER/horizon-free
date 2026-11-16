//! Cold authoring access to opaque entity type tokens.
/// Construct a registration-owned token.
pub fn entity_type_key(slot: u32) -> crate::EntityTypeKey {
    crate::EntityTypeKey(slot)
}
/// Its opaque slot for round-trip assertions.
pub fn entity_type_key_slot(key: crate::EntityTypeKey) -> u32 {
    key.0
}
/// Validate membership in the installed lookup.
#[cfg(any(test, feature = "testing"))]
pub fn validate_entity_type_key(slot: u32) -> Result<crate::EntityTypeKey, &'static str> {
    crate::EntityTypeKey::from_slot(slot)
}
