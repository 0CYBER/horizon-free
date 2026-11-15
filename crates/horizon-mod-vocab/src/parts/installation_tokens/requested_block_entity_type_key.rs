//! Narrow identity reads for API-owned fixtures.
/// Look up only the requested identity, without exposing its owning table.
#[cfg(any(test, feature = "testing"))]
pub fn requested_block_entity_type_key(id: &str) -> Option<crate::BlockEntityTypeKey> {
    crate::parts::requested_keys()
        .block_entity_types
        .slots
        .get(id)
        .copied()
        .map(crate::BlockEntityTypeKey)
}
