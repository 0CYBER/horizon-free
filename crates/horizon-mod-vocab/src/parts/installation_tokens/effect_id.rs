//! Scalar identity read for API fixtures.
/// The installed name, if resolved.
#[cfg(any(test, feature = "testing"))]
pub fn effect_id(key: crate::EffectKey) -> Option<&'static str> {
    crate::content::registry_keys::effect_id(key)
}
