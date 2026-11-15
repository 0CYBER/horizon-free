use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A mob converted into another type (NeoForge's
    /// `LivingConversionEvent.Post`): `converted` replaces `entity`. Fires
    /// with `native_entity_transform`, after the finalized new mob joined the
    /// level and before the old mob is removed, for the conversions
    /// `native_living_conversion_pre` tells. API 390.
    pub(in crate::game) fn native_living_conversion_post(
        &mut self,
        entity: u32,
        kind: &str,
        converted: u32,
        converted_kind: &str,
    ) {
        self.fire_living_conversion_post(entity, kind, converted, converted_kind);
    }
}
