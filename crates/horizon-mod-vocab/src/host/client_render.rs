//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_render` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ClientRenderClientHost {
    /// Atomically overwrite retained model copies; use [`InstanceGroupRef::write`].
    #[access(PublishedResources)]
    fn instance_group_write(
        &self,
        group: InstanceGroupRef,
        first: u32,
        instances: &[ModelInstance],
    ) -> Result<(), GroupError>;
    /// Set the drawn length; use [`InstanceGroupRef::set_len`].
    #[access(PublishedResources)]
    fn instance_group_set_len(&self, group: InstanceGroupRef, len: u32) -> Result<(), GroupError>;
    /// Replace the group pose; use [`InstanceGroupRef::set_group_transform`].
    #[access(PublishedResources)]
    fn instance_group_set_transform(
        &self,
        group: InstanceGroupRef,
        transform: [f32; 12],
    ) -> Result<(), GroupError>;
    /// Retire retained model copies; use [`InstanceGroupRef::destroy`].
    #[access(PublishedResources)]
    fn instance_group_destroy(&self, group: InstanceGroupRef) -> Result<(), GroupError>;
    /// Side: client.
    /// Asks `block_render_visibility` again for every state and, when the answers differ
    /// from those in use, remeshes every loaded section with them. Call it when your answer changes (a toggle key, a scan
    /// finishing) and once when the player joins, not every frame: a rebuild takes the workers a
    /// few seconds. With no part answering, nothing is asked and the mesher keeps its no-mod path.
    #[access(WorldOnly)]
    fn refresh_render_visibility(&mut self);
    /// Side: client.
    /// Asks `biome_colors` again for every biome and, when the colours
    /// differ from those in use, rebuilds every loaded section and the far
    /// terrain with them (`LevelRenderer.allChanged`, as changing Biome Blend
    /// does). Call it when your colours change (a new season, a new day of a
    /// slow fade) and once when the player joins, not every frame: a rebuild
    /// takes the workers a few seconds.
    #[access(WorldOnly)]
    fn refresh_biome_colors(&mut self);
    /// Side: client.
    /// A mod's own RGBA image, uploaded into the HUD atlas slot
    /// `MOD_IMAGE_SPRITE` and drawn with
    /// `native_sprite`: its own source rectangle crops and stretches it. `rgba`
    /// is `width * height` pixels of four bytes, each side at most
    /// `MOD_IMAGE_SIZE`; false when it is not that
    /// size. The upload costs a texture patch of the image, so send one when the
    /// image changed, not every frame.
    ///
    /// The slot exists whenever a client mod, native or wasm, is loaded: the atlas packs it at
    /// load and on a resource reload, with no mixin and no opt-in. It is one image for all mods
    /// together: a second mod's write replaces the first's.
    #[access(WorldOnly)]
    fn mod_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool;
    /// Side: client.
    /// Vanilla's `TextureManager.register(id, new DynamicTexture(...))`, without a resource
    /// reload: the calling mod's image `id` from `rgba`, `width * height` pixels of four bytes, row
    /// by row from the top left, placed in the mod's half of the GUI atlas's page for dynamic
    /// textures (a mod's first image picks the half fewer mods use, so a second mod gets the half
    /// the first does not). `id` names it as a GUI sprite of `width` x `height` pixels:
    /// `DrawContext::sprite`, an inline `TextObject::Atlas` of `minecraft:gui` and
    /// [`gui_sprite`](Self::gui_sprite) draw and find it as they do any GUI sprite, and its pixels
    /// reach the GPU with the next frame. It lasts, through resource reloads, until
    /// [`free_dynamic_texture`](Self::free_dynamic_texture) or the game's end.
    ///
    /// Refused by name, with nothing changed and nothing allocated, as [`DynamicTextureError`]
    /// lists: `Unavailable` when the atlas has no page, `NotOwned` for an id outside the mod's
    /// namespaces, `BadSize` for a side of 0 or pixels that are not `width * height * 4` bytes,
    /// `TooLarge` for a side past [`DYNAMIC_TEXTURE_MAX_SIDE`], `NameTaken` for an id that
    /// already names an image or a GUI sprite in either of its forms (`ns:path` and
    /// `ns:gui/sprites/path`), `OverBudget` past the mod's [`DYNAMIC_TEXTURE_MOD_IMAGES`] images
    /// or [`DYNAMIC_TEXTURE_MOD_BYTES`] of pixels, and `NoSlot` when the mod's half of the page
    /// has no room left for the image. The game keeps a copy of the pixels, counted in the byte
    /// budget, to upload them again into a rebuilt atlas.
    #[access(WorldOnly)]
    fn create_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// Vanilla's `DynamicTexture.upload` of new pixels: the calling mod's image `id` takes
    /// `rgba`, `width * height` pixels of four bytes, from the next frame on. The size may change:
    /// the sprite `id` takes the new size, in the place the image holds when it fits there, or in
    /// a new one (then the mod's half needs room for the new size beside the old). Each update is
    /// one copy into the kept pixels and one upload of the image, so send one when the image
    /// changed, not every frame.
    ///
    /// Refused by name, with nothing changed, as [`DynamicTextureError`] lists: `Unavailable`,
    /// `NotOwned`, `Unknown` for an id that names no image, `BadSize`, `TooLarge`, `OverBudget`
    /// when the new size would pass [`DYNAMIC_TEXTURE_MOD_BYTES`], and `NoSlot` when a larger
    /// image finds no room.
    #[access(WorldOnly)]
    fn update_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// Vanilla's `TextureManager.release`: frees the calling mod's image `id`. The GUI sprite
    /// `id` is gone at once, its room goes back to the mod's half of the page, and the image and
    /// its bytes go back to the mod's budget. Refused by name, with nothing changed:
    /// `Unavailable`, `NotOwned` for an id outside the mod's namespaces or another mod's image,
    /// and `Unknown` for one that names no image.
    #[access(WorldOnly)]
    fn free_dynamic_texture(&mut self, id: &Identifier) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// What the calling mod may still create: images and bytes of pixels left of its
    /// [`DYNAMIC_TEXTURE_MOD_IMAGES`] and [`DYNAMIC_TEXTURE_MOD_BYTES`]. These are limits of the
    /// atlas page, which vanilla's `TextureManager` does not have. The room left in the mod's half
    /// of the page is not part of it. `Unavailable` when the atlas has no page, `NotOwned` when no
    /// mod is calling.
    #[access(WorldOnly)]
    fn dynamic_texture_budget(&self) -> Result<DynamicTextureBudget, DynamicTextureError>;
    /// Side: client.
    /// For a client mod drawing a block's faces itself (a copycat drawing its material's, from
    /// `block_sprites`): the 0xRRGGBB colour a face of `state` with tint index `tint_index`
    /// (`FaceSprite::tint_index`) is multiplied by at `pos`, as the chunk mesher tints it there
    /// (`BlockColors.getColor`, biome blend included: grass, foliage and water follow the biome
    /// around `pos`). None for an untinted face (-1, or an index past the state's tints). Reads
    /// this client's copy of the world.
    #[access(WorldOnly)]
    fn block_tint_color(&self, state: BlockState, tint_index: i32, pos: [i32; 3]) -> Option<u32>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: what each resolved block state draws on its six sides, preserving
    /// its properties without reparsing names. Resolve states once at load. Each face's sprite (array and
    /// layer, as `WorldDrawn::block_sprites` cuts them) and its texture by the name `cube` and `quad` take,
    /// where it lies on the side, how its texture runs across it (`FaceSprite::quad_on` draws it on another
    /// block's box, turned as its model turns it), its tint source and index (`block_tint_color`) and
    /// whether it is cut out, translucent or animated; its particle sprite; whether it is a plain
    /// cube. None for a block the game does not know. The model is the state's first random variant. API
    /// 380.
    #[access(PublishedResources)]
    fn block_sprites(&self, blocks: &[BlockState]) -> Vec<Option<BlockSprites>>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: each texture's sprite by name (`minecraft:block/stone`, `block/stone`), none for one no
    /// model uses. API 380.
    #[access(PublishedResources)]
    fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<SpriteRef>>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod reading the player's own block textures (resource packs included) on the CPU: a map
    /// averaging each block's top colour, a software renderer. Each sprite's (`block_sprites`, `texture_sprites`)
    /// RGBA8, rows top to bottom, square, every one the arrays' edge (`rgba.len()` is edge × edge × 4): the pixels
    /// the arrays were filled with when the resources loaded, so an animated sprite's is the frame it starts on.
    /// None for a sprite past the arrays' layers, and for all of them on a client that loaded no pixels. The
    /// pixels are kept compressed once the GPU has them: one call uncompresses them all once, so ask for every
    /// sprite at once and again only when `block_sprites_version` changes. `Err`, reading nothing, for more than
    /// 256 MiB of pixels in one call (`MAX_SPRITE_PIXEL_BYTES`, the default texture budget: every layer fits
    /// unless the player raised `textureBudgetMiB`; ask in parts then).
    #[access(PublishedResources)]
    fn sprite_pixels(&self, sprites: &[SpriteRef]) -> Result<Vec<Option<Vec<u8>>>, String>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: where GUI sprite `sprite` lies in the GUI atlas and its size, none for one the atlas lacks
    /// (a server's metrics-only atlas lacks them all). The atlas names vanilla's sprites by path alone
    /// (`gui/sprites/container/slot`) and a mod's with its namespace (`mymod:gui/panel`).
    #[access(PublishedResources)]
    fn gui_sprite(&self, sprite: &Identifier) -> Option<AtlasRegion>;
    /// Side: client.
    /// For a client mod drawing a player's model itself (a 3D map, a portrait): where the player's
    /// skin and cape lie in the GUI atlas, and whether the model is slim, as this client draws that
    /// player: the downloaded skin once it is in the atlas, the profile's default until then. The
    /// regions hold for this frame (a later download or another player's skin can take a slot),
    /// so ask each frame you draw. None for an entity that is not a player or a mannequin this
    /// client shows, and for a stale handle.
    #[access(WorldOnly)]
    fn player_skin(&self, entity: EntityRef) -> Option<PlayerSkin>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: changes each time the resources load (a resource pack turned on or off can move every
    /// sprite): ask `native_block_sprites` again then. API 380.
    #[access(PublishedResources)]
    fn block_sprites_version(&self) -> u64;
}
