//! `ClientRenderClientHost::create_dynamic_texture`, `update_dynamic_texture`,
//! `free_dynamic_texture` and `dynamic_texture_budget` (#11138): a client mod's dynamic textures
//! (vanilla's `DynamicTexture` in the `TextureManager`), held in the GUI atlas's page for the mod
//! whose hook or part is running (`crash::current_native_mod`) and named among the atlas's
//! sprites, so `native_sprite`, `gui_sprite` and inline atlas text draw them with no resource
//! reload. `game.dynamic_textures` outlives the atlas: a resource reload rebinds every image
//! (`world/packs`), a new renderer patches each again (`forget_atlas_slots`), and each frame's
//! HUD batch uploads what changed (`game.rs`, beside the map patches).
use super::Game;
use mod_api::{DynamicTextureBudget, DynamicTextureError, Identifier};

impl Game<'_> {
    /// The calling mod's image `id` made from `rgba` (`width * height` pixels of four bytes) in
    /// the atlas's page, its pixels uploaded with the next frame.
    pub(in crate::game) fn create_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let owner = mod_api::content::crash::current_native_mod();
        self.dynamic_textures
            .create(&mut self.gui.atlas, owner, id, [width, height], rgba)
    }
    /// The calling mod's image `id` given new pixels, uploaded with the next frame.
    pub(in crate::game) fn update_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let owner = mod_api::content::crash::current_native_mod();
        self.dynamic_textures
            .update(&mut self.gui.atlas, owner, id, [width, height], rgba)
    }
    /// The calling mod's image `id` freed: its name and room go at once.
    pub(in crate::game) fn free_dynamic_texture(
        &mut self,
        id: &Identifier,
    ) -> Result<(), DynamicTextureError> {
        let owner = mod_api::content::crash::current_native_mod();
        self.dynamic_textures.free(&mut self.gui.atlas, owner, id)
    }
    /// What the calling mod may still create.
    pub(in crate::game) fn dynamic_texture_budget(
        &self,
    ) -> Result<DynamicTextureBudget, DynamicTextureError> {
        let owner = mod_api::content::crash::current_native_mod();
        self.dynamic_textures.budget(&self.gui.atlas, owner)
    }
}
