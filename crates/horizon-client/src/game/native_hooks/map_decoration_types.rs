use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The map decoration types a mod adds (NeoForge's `RegisterEvent` for
    /// `Registries.MAP_DECORATION_TYPE`): push a `MapDecorationType` per type. Asked once at
    /// start, on every side.
    pub(in crate::game) fn native_map_decoration_types(
        context: &mod_api::RegistrationContext,
        types: &mut Vec<MapDecorationType>,
    ) {
        Self::fire_map_decoration_types(context, types);
    }
}
