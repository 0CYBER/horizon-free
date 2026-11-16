//! Construction for API-owned fixtures.
/// Build the typed value without exposing its native representation.
#[cfg(any(test, feature = "testing"))]
pub fn overanswered(owner: String) -> crate::Overanswered {
    crate::Overanswered { owner }
}
