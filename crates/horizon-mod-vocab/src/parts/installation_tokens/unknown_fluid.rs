//! Construction for API-owned fixtures.
/// Build the typed value without exposing its native representation.
#[cfg(any(test, feature = "testing"))]
pub fn unknown_fluid(mod_id: &'static str, id: String) -> crate::UnknownFluid {
    crate::UnknownFluid { mod_id, id }
}
