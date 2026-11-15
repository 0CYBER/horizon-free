//! Capability attribution for API fixture assertions.
/// Whether this answer retains a mod-owned dispatch guard.
#[cfg(any(test, feature = "testing"))]
pub fn capability_is_owned<T: ?Sized>(capability: &crate::Capability<T>) -> bool {
    capability.owner.is_some()
}
