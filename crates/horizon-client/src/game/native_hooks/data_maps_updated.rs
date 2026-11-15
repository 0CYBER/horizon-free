use super::*;

impl Game<'_> {
    /// The data maps of `registry` (`minecraft:item`) changed on this side
    /// (NeoForge's `DataMapsUpdatedEvent`), after they are readable.
    pub(in crate::game) fn native_data_maps_updated(
        &mut self,
        registry: &str,
        cause: mod_api::DataMapsUpdateCause,
    ) {
        let phase = self.mod_phase.clone();
        let _reload = phase
            .reloading()
            .expect("data maps notice on its owning thread");
        self.fire_data_maps_updated(registry, cause);
    }
}
