//! Erase a validated capability provider without exposing its native fields.
/// Construct the vocabulary's erased provider representation.
pub fn item_provider<T: ?Sized + 'static>(
    provider: std::sync::Arc<dyn crate::ItemCapabilityProvider<T>>,
) -> crate::ItemProviderOf {
    crate::ItemProviderOf::of(provider)
}
