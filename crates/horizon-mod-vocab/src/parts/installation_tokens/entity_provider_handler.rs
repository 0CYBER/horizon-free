//! Erased provider metadata retained behind the native vocabulary enum.
/// Its validated handler name for registration collision checks.
pub fn entity_provider_handler(provider: &crate::EntityProviderOf) -> std::sync::Arc<str> {
    provider.handler()
}
