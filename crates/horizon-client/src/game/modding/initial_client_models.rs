//! Initial-load publication boundary shared by startup and its resource fixture.
use super::Game;
use crate::game::Result;

impl Game<'_> {
    /// Publishes the initial load to the mods: entity renderers, atlases, baked models, then
    /// `client_setup`.
    ///
    /// The renderers resolve against the mob types, which live in the world resources that the
    /// `world-rest-loader` worker fills while the title is shown. They are joined first: without it a
    /// launch that reaches this point before the worker finishes panics in `Resources::rest`
    /// ("world resources joined before entry"). Joining cannot be left for later: the mods hear
    /// `client_setup` here, once, before their configs start and before any screen, so a failed
    /// load stops startup with its error instead of skipping the setup.
    pub(in crate::game) fn publish_initial_client_models(&mut self) -> Result<()> {
        if self.options.is_client_start() {
            self.join_world_rest()?;
            self.resolve_entity_renderers();
            self.texture_atlases_stitched();
            self.native_model_baking_completed();
            self.native_client_setup();
        }
        Ok(())
    }
}
