//! Erased provider metadata retained behind the native vocabulary enum.
/// Its validated handler name for registration collision checks.
pub fn item_provider_handler(provider: &crate::ItemProviderOf) -> std::sync::Arc<str> {
    provider.handler()
}
