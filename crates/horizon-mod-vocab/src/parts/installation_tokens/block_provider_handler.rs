//! Erased provider metadata retained behind the native vocabulary enum.
/// Its validated handler name for registration collision checks.
pub fn block_provider_handler(provider: &crate::BlockProviderOf) -> std::sync::Arc<str> {
    provider.handler()
}
