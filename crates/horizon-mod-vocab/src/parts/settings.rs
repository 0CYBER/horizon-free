use super::*;
/// Owner-bound settings retained for the client's mod pages and typed reads.
pub fn settings() -> &'static [(&'static str, crate::SettingSpec)] {
    installed().map_or(&[], |installed| &installed.keys.settings)
}

/// The setting `key` mod `mod_id` registered, if any.
pub fn setting(mod_id: &str, key: &str) -> Option<&'static crate::SettingSpec> {
    settings()
        .iter()
        .find(|(owner, spec)| *owner == mod_id && spec.key == key)
        .map(|(_, spec)| spec)
}

/// Registrar operations for settings.
pub trait SettingsRegistrar {
    /// Register a setting shown on the calling mod's page in Options > Mods (NeoForge's
    /// `ModConfigSpec` entries). Called from `Hooks::register`; client settings only. A setting
    /// [`SettingSpec::check`](crate::SettingSpec::check) refuses, or a key the mod registered
    /// already, is not registered and comes back as the error.
    fn register_setting(&mut self, setting: crate::SettingSpec) -> Result<(), crate::SettingError>;
}
