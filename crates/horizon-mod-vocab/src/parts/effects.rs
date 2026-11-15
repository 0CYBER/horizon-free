use super::*;
pub(crate) fn resolved_effects() -> &'static OnceLock<horizon_mod_runtime::registry_keys::Effects> {
    static EMPTY: OnceLock<horizon_mod_runtime::registry_keys::Effects> = OnceLock::new();
    installed().map_or(&EMPTY, |installed| &installed.effects)
}

/// Registrar operations for effects.
pub trait EffectsRegistrar {
    /// Request an effect by canonical registry id. All mods' declarations are checked at load.
    fn effect_key(&mut self, id: &str) -> crate::EffectKey;
}
