//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// A face's tint source as the game colours it (`BlockColors.createDefault`), for drawing a block
/// elsewhere ([`crate::block::block_sprites`]): the biome's grass, foliage, dry foliage or water colour
/// where the block is, or a fixed colour (0xRRGGBB).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TintSource {
    Grass,
    Foliage,
    DryFoliage,
    Water,
    Fixed(u32),
}
