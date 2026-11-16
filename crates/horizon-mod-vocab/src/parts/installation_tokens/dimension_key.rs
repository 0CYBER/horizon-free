//! Opaque scalar identities for registration contract fixtures.
/// Construct an explicit test token.
#[cfg(any(test, feature = "testing"))]
pub fn dimension_key(slot: u32) -> crate::DimensionKey {
    crate::DimensionKey(slot)
}
/// Its opaque slot for round-trip assertions.
#[cfg(any(test, feature = "testing"))]
pub fn dimension_key_slot(key: crate::DimensionKey) -> u32 {
    key.0
}
/// Validate membership in the installed lookup.
#[cfg(any(test, feature = "testing"))]
pub fn validate_dimension_key(slot: u32) -> Result<crate::DimensionKey, &'static str> {
    crate::DimensionKey::from_slot(slot)
}
