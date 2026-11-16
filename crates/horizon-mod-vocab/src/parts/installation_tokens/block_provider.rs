//! Erase a validated capability provider without exposing its native fields.
/// Construct the vocabulary's erased provider representation.
pub fn block_provider<T: ?Sized + 'static>(
    provider: std::sync::Arc<dyn crate::BlockCapabilityProvider<T>>,
) -> crate::BlockProviderOf {
    crate::BlockProviderOf::of(provider)
}
