//! Narrow identity reads for API-owned fixtures.
/// Look up only the requested identity, without exposing its owning table.
#[cfg(any(test, feature = "testing"))]
pub fn resolved_effect_key(id: &str) -> Option<crate::EffectKey> {
    crate::parts::resolved_effects()
        .get()?
        .names
        .slots
        .get(id)
        .copied()
        .map(crate::EffectKey)
}
