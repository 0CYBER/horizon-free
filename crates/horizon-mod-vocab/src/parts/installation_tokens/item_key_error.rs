//! Construction for API-owned fixtures.
/// Build the typed value without exposing its native representation.
#[cfg(any(test, feature = "testing"))]
pub fn item_key_error(mod_id: &'static str, id: String) -> crate::parts::ItemKeyError {
    crate::parts::ItemKeyError { mod_id, id }
}
