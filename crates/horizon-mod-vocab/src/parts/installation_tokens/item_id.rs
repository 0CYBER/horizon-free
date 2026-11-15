//! Scalar identity read for API fixtures.
/// The installed name, if resolved.
#[cfg(any(test, feature = "testing"))]
pub fn item_id(key: crate::ItemKey) -> Option<&'static str> {
    crate::content::registry_keys::item_id(key)
}
