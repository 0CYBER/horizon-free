//! Registered mob effect classification.

/// Vanilla `MobEffectCategory`, read from a registered effect's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum EffectCategory {
    /// A beneficial effect, such as speed.
    Beneficial,
    /// A harmful effect, such as poison.
    Harmful,
    /// Neither beneficial nor harmful, such as glowing.
    Neutral,
}

impl crate::EffectKey {
    /// Reads `MobEffect.getCategory()` for this registered type on either side.
    /// Returns `None` before the effect registry is complete.
    /// An active effect's `id` is its type key and supports the same read.
    pub fn category(self, host: &dyn crate::Host) -> Option<EffectCategory> {
        crate::backend_of(host)?.effect_category(self)
    }
}
