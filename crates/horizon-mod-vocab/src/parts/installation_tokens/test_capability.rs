//! Capability construction for isolated API fixture failures.
/// Attribute a synthetic handler without exposing the native capability fields.
#[cfg(any(test, feature = "testing"))]
pub fn test_capability<T: ?Sized>(
    handler: Box<T>,
    owner: Option<&'static str>,
) -> crate::Capability<T> {
    crate::Capability { handler, owner }
}
