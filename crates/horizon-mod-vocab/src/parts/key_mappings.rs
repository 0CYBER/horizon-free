use super::*;
/// Owner-bound declarations retained for client mod-list rebuilds.
pub fn key_mappings() -> &'static [(&'static str, crate::KeyRegistration)] {
    installed().map_or(&[], |installed| &installed.keys.key_mappings)
}

/// Registrar operations for key mappings.
pub trait KeyMappingsRegistrar {
    /// Register a Controls binding owned by the calling mod (`RegisterKeyMappingsEvent`).
    /// Called from `Hooks::register`; invalid or duplicate local ids refuse startup.
    fn register_key_mapping(&mut self, mapping: crate::KeyRegistration);
}
