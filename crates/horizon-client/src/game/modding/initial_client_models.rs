//! Initial-load publication boundary shared by startup and its resource fixture.
use super::Game;
impl Game<'_> {
    pub(in crate::game) fn publish_initial_client_models(&mut self) {
        if self.options.is_client_start() {
            self.resolve_entity_renderers();
            self.texture_atlases_stitched();
            self.native_model_baking_completed();
            self.native_client_setup();
        }
    }
}
