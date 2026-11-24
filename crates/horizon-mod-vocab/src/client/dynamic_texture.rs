//! A client mod's dynamic textures (`ClientRenderClientHost::create_dynamic_texture`,
//! `update_dynamic_texture`, `free_dynamic_texture` and `dynamic_texture_budget`, vanilla's
//! `DynamicTexture` registered in the `TextureManager`): RGBA images kept in one page of the GUI
//! atlas, each mod's in its own half of it, within a budget per mod; what a mod has left of it,
//! and why a call was refused. The names and signatures are Sunrise's (sunrise-org/sunrise), and
//! so are the byte budget and the longest side; the page and the image count are Horizon's, and
//! Sunrise's size classes and its `ClassChange` refusal have no counterpart here.

/// The longest side of a dynamic texture: a longer one is [`DynamicTextureError::TooLarge`].
pub const DYNAMIC_TEXTURE_MAX_SIDE: u32 = 1024;
/// The most dynamic textures one mod holds at once ([`DynamicTextureError::OverBudget`]): an
/// emoji set of 32 x 32 images fills it with the byte budget below.
pub const DYNAMIC_TEXTURE_MOD_IMAGES: u32 = 1024;
/// The most bytes of pixels (`width * height * 4` an image) one mod's dynamic textures hold
/// together, 4 MiB ([`DynamicTextureError::OverBudget`]): one 1024 x 1024 image, or 1024 images
/// of 32 x 32. The game keeps each image's last pixels in memory to upload them again after a
/// resource reload, and this is what they may take.
pub const DYNAMIC_TEXTURE_MOD_BYTES: u64 = 4 << 20;

/// What the calling mod may still create: images and bytes of pixels left of its budget
/// ([`DYNAMIC_TEXTURE_MOD_IMAGES`], [`DYNAMIC_TEXTURE_MOD_BYTES`]). It does not say whether the
/// mod's half of the atlas page has room for an image: a half with no space for it is
/// [`DynamicTextureError::NoSlot`]. Only the game makes one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct DynamicTextureBudget {
    /// Images the mod may still create.
    pub images: u32,
    /// Bytes of pixels its images may still add, four a pixel.
    pub bytes: u64,
}

/// Why a dynamic texture call changed nothing. The mod's images, every other mod's and the
/// atlas are as they were before the call.
///
/// The cases are part of the calls' wire fingerprint and are fixed once published: adding,
/// renaming or reordering one refuses every wasm mod built before the change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DynamicTextureError {
    /// The id is not in a namespace owned by the calling mod (`mymod:emoji/smile` for the mod
    /// `mymod`, or its manifest's other namespaces), the id is `minecraft:`, the image is another
    /// mod's, or no mod is calling: a mod creates, updates and frees only its own images.
    NotOwned,
    /// A side is 0, or the pixels are not `width * height * 4` bytes.
    BadSize,
    /// A side is longer than [`DYNAMIC_TEXTURE_MAX_SIDE`].
    TooLarge,
    /// The mod's half of the atlas page has no free space of the image's size left (each image
    /// takes one more texel each way, a transparent gap). Its own images took it, or those of
    /// mods that share its half, which happens only when both halves held other mods' images as
    /// a mod made its first: free one, or make the image smaller.
    NoSlot,
    /// The mod would hold more than [`DYNAMIC_TEXTURE_MOD_IMAGES`] images or
    /// [`DYNAMIC_TEXTURE_MOD_BYTES`] of pixels: free some first (`dynamic_texture_budget` says
    /// what is left).
    OverBudget,
    /// `create_dynamic_texture` of an id that already names an image or a sprite of the GUI atlas,
    /// in either of its forms (`mymod:emoji/smile` and `mymod:gui/sprites/emoji/smile`, which an
    /// inline atlas object finds for the same id): update it, or choose another id.
    NameTaken,
    /// `update_dynamic_texture` or `free_dynamic_texture` of an id that names no image.
    Unknown,
    /// The atlas has no page for dynamic textures: nothing draws here (a server's thread, a
    /// dedicated server, no window), or no client mod was loaded when the atlas was built.
    Unavailable,
}

impl std::fmt::Display for DynamicTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotOwned => "the id is not in the calling mod's namespace",
            Self::BadSize => "a side is 0 or the pixels are not width * height * 4 bytes",
            Self::TooLarge => "a side is longer than DYNAMIC_TEXTURE_MAX_SIDE",
            Self::NoSlot => "the mod's half of the page has no free space of the image's size",
            Self::OverBudget => "the calling mod's dynamic textures would pass its budget",
            Self::NameTaken => "the id already names an image or a GUI sprite",
            Self::Unknown => "no dynamic texture has this id",
            Self::Unavailable => "the GUI atlas has no dynamic texture page here",
        })
    }
}

impl std::error::Error for DynamicTextureError {}

impl crate::NoBackend for DynamicTextureError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
