//! Erase a validated capability provider without exposing its native fields.
/// Construct the vocabulary's erased provider representation.
pub fn entity_provider<T: ?Sized + 'static, C: 'static>(
    provider: std::sync::Arc<dyn crate::EntityCapabilityProvider<T, C>>,
) -> crate::EntityProviderOf {
    crate::EntityProviderOf::of(provider)
}
