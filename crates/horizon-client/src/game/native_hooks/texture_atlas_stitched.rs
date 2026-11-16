use super::*;

impl Game<'_> {
    /// `TextureAtlasStitchedEvent` for each atlas the resources just published, the blocks' then
    /// the GUI's: on the first load before `client_setup`, after a successful reload before
    /// `client_resources_reloaded`.
    pub(in crate::game) fn texture_atlases_stitched(&mut self) {
        for atlas in [mod_api::TextureAtlas::Blocks, mod_api::TextureAtlas::Gui] {
            self.native_texture_atlas_stitched(atlas);
        }
    }
    /// Atlas `atlas` was stitched and published: its sprites can be looked up.
    pub(in crate::game) fn native_texture_atlas_stitched(&mut self, atlas: mod_api::TextureAtlas) {
        self.fire_texture_atlas_stitched(atlas);
    }
}
