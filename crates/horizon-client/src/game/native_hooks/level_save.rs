use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `dimension`'s level was saved (NeoForge's `LevelEvent.Save`, in
    /// `ServerLevel.save`): its level data, chunks and entities, by the
    /// autosave, `/save-all`, a pause, a closing level or the server
    /// stopping; each level of a world save in turn. Server-side.
    pub(in crate::game) fn native_level_save(&mut self, dimension: &str) {
        self.fire_level_save(dimension);
    }
}
