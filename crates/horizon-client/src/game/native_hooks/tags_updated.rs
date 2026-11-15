use super::*;

impl Game<'_> {
    /// Tags changed on this side (NeoForge's `TagsUpdatedEvent`), after they are readable.
    pub(in crate::game) fn native_tags_updated(&mut self, cause: mod_api::TagsUpdateCause) {
        let phase = self.mod_phase.clone();
        let _reload = phase.reloading().expect("tags notice on its owning thread");
        self.fire_tags_updated(cause);
    }
}
