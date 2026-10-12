//! Placed banners (`BannerBlockEntity`): the pattern layers move from the
//! banner item to the block when placed and back to the dropped item when
//! broken (`copy_components` in the banner loot tables).
use super::Game;
use horizon_client::{
    banner::{self, BannerLayer},
    containers::{Banner, BlockEntity},
    items::ItemStack,
};

impl Game<'_> {
    /// The pattern textures and tints of the banner at `pos`, bottom first.
    pub(super) fn banner_layers_at(&self, pos: [i32; 3]) -> Vec<(String, u32)> {
        let Some(BlockEntity::Banner(b)) = self.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        else {
            return Vec::new();
        };
        layer_textures(&self.res.stations.banners, &b.layers)
    }
    /// A placed banner takes the item's layers and name.
    pub(super) fn fill_placed_banner(&mut self, pos: [i32; 3], stack: &ItemStack) {
        self.pending_banner.remove(&pos);
        if !stack.item.ends_with("_banner") {
            return;
        }
        if let Some(BlockEntity::Banner(b)) =
            self.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.layers = banner::layers(stack).to_vec();
            b.name = stack.custom_name().map(str::to_owned);
        }
    }
    /// The layers a banner dropped at `pos` carries.
    pub(super) fn attach_banner_layers(&mut self, pos: [i32; 3], stack: &mut ItemStack) {
        if !stack.item.ends_with("_banner") {
            return;
        }
        if let Some(Banner { layers, name }) = self.pending_banner.remove(&pos) {
            let data = stack.data_mut();
            data.banner_patterns = layers;
            if name.is_some() {
                data.custom_name = name;
            }
            stack.tidy();
        }
    }
}

/// Texture names and tints of banner layers (`Sheets.getBannerSprite`).
pub(super) fn layer_textures(
    banners: &banner::Banners,
    layers: &[BannerLayer],
) -> Vec<(String, u32)> {
    layers
        .iter()
        .map(|l| {
            (
                banners.banner_texture(&l.pattern),
                banner::dye_rgb(&l.color),
            )
        })
        .collect()
}
