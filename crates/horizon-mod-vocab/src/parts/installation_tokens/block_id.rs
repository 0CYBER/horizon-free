//! Scalar identity read for API fixtures.
/// The installed name, if resolved.
#[cfg(any(test, feature = "testing"))]
pub fn block_id(key: crate::BlockKey) -> Option<&'static str> {
    crate::content::registry_keys::block_id(key)
}
