//! Narrow identity reads for API-owned fixtures.
/// Look up only the requested identity, without exposing its owning table.
#[cfg(any(test, feature = "testing"))]
pub fn requested_block_key(id: &str) -> Option<crate::BlockKey> {
    crate::parts::requested_keys()
        .blocks
        .slots
        .get(id)
        .copied()
        .map(crate::BlockKey)
}
