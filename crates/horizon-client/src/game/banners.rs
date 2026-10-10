//! Placed banners (`BannerBlockEntity`): the pattern layers move from the
//! banner item to the block when placed and back to the dropped item when
//! broken (`copy_components` in the banner loot tables).
use super::Game;
use glam::{EulerRot, Mat4, Quat, Vec3};
use horizon_client::{
    banner::{self, BannerLayer},
    block_entity_render::{Paint, draw_part},
    containers::{Banner, BlockEntity},
    hud::{HudBatch, rgba},
    items::ItemStack,
    mesher::Vertex,
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
    /// A banner in a slot: `item/template_banner`'s `gui` transform on the
    /// banner model (pole, bar and flag with every layer), flat-lit and
    /// depth-sorted. False when the stack is not a banner.
    pub(super) fn draw_banner_icon(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
    ) -> bool {
        let Some(base) = banner::banner_color(&stack.item) else {
            return false;
        };
        let Some((main, flag)) = self.res.block_entity_render.banner_models() else {
            return true;
        };
        let banners = &self.res.stations.banners;
        let sprites = &self.res.atlas.sprites;
        // display.gui, then the item's special transformation (a banner is
        // modelled upside down and 2/3 size around the block centre).
        let m = Mat4::from_translation(Vec3::new(0.0, -3.25 / 16.0, 0.0))
            * Mat4::from_quat(Quat::from_euler(
                EulerRot::XYZ,
                30f32.to_radians(),
                20f32.to_radians(),
                0.0,
            ))
            * Mat4::from_scale(Vec3::splat(0.5325))
            * Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0))
            * Mat4::from_scale(Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0));
        let wave = |name: &str, p: &mut [f32; 9]| {
            if name == "flag" {
                p[3] = (-0.0125 + 0.01) * std::f32::consts::PI;
            }
        };
        let none = |_: &str, _: &mut [f32; 9]| {};
        let mut passes: Vec<(&str, u32, bool, String)> = vec![
            ("entity/banner/banner_base", 0xFFFFFF, true, String::new()),
            (
                "entity/banner/base",
                banner::dye_rgb(base),
                false,
                String::new(),
            ),
        ];
        for layer in banner::layers(stack).iter().take(16) {
            passes.push((
                "",
                banner::dye_rgb(&layer.color),
                false,
                banners.banner_texture(&layer.pattern),
            ));
        }
        // (depth, pass, screen corners, uv, colour)
        let mut faces: Vec<(f32, usize, [[f32; 2]; 4], [[f32; 2]; 4], u32)> = Vec::new();
        for (index, (fixed, tint, with_pole, dynamic)) in passes.iter().enumerate() {
            let name = if fixed.is_empty() {
                dynamic.as_str()
            } else {
                fixed
            };
            let Some(sprite) = sprites.get(name).copied() else {
                continue;
            };
            let paint = Paint {
                sprite,
                tint: 0xFFFFFF,
                light: 0,
                lights: (Vec3::ZERO, Vec3::ZERO),
            };
            let mut verts: Vec<Vertex> = Vec::new();
            if *with_pole {
                draw_part(main, "root", m, &none, paint, &mut verts);
            }
            draw_part(flag, "root", m, &wave, paint, &mut verts);
            for quad in verts.as_chunks::<4>().0 {
                let p = |i: usize| Vec3::from(quad[i].pos);
                let normal = (p(1) - p(0)).cross(p(2) - p(0)).normalize_or_zero();
                // Faces turned away from the viewer are culled.
                if normal.z <= 0.0 {
                    continue;
                }
                let depth = (0..4).map(|i| p(i).z).sum::<f32>() / 4.0;
                let shade = (0.6 + 0.4 * normal.z).min(1.0);
                let c = |s: u32| (((tint >> s) & 255) as f32 * shade) as u8;
                faces.push((
                    depth,
                    index,
                    std::array::from_fn(|i| {
                        [
                            x + 8.0 + quad[i].pos[0] * 16.0,
                            y + 8.0 - quad[i].pos[1] * 16.0,
                        ]
                    }),
                    std::array::from_fn(|i| quad[i].uv),
                    rgba(c(16), c(8), c(0), 255),
                ));
            }
        }
        faces.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, _, corners, uv, color) in faces {
            batch.quad(corners, uv, color);
        }
        true
    }
    /// A banner in the first-person hand: the standing banner model under the
    /// item's `firstperson_righthand` transform (`m`) and its special
    /// transformation. False when the stack is not a banner.
    pub(super) fn held_banner_geometry(
        &self,
        m: Mat4,
        stack: &ItemStack,
        packed: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        let Some(base) = banner::banner_color(&stack.item) else {
            return false;
        };
        let Some((main, flag)) = self.res.block_entity_render.banner_models() else {
            return true;
        };
        let m = m
            * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
            * Mat4::from_scale(Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0));
        let wave = |name: &str, p: &mut [f32; 9]| {
            if name == "flag" {
                p[3] = (-0.0125 + 0.01) * std::f32::consts::PI;
            }
        };
        let none = |_: &str, _: &mut [f32; 9]| {};
        let lights = horizon_client::block_entity_render::diffuse_lights(false);
        let sprites = &self.res.atlas.sprites;
        let paint = |name: &str, tint: u32| {
            sprites.get(name).map(|&sprite| Paint {
                sprite,
                tint,
                light: packed,
                lights,
            })
        };
        if let Some(p) = paint("entity/banner/banner_base", 0xFFFFFF) {
            draw_part(main, "root", m, &none, p, out);
            draw_part(flag, "root", m, &wave, p, out);
        }
        if let Some(p) = paint("entity/banner/base", banner::dye_rgb(base)) {
            draw_part(flag, "root", m, &wave, p, out);
        }
        for layer in banner::layers(stack).iter().take(16) {
            let texture = self.res.stations.banners.banner_texture(&layer.pattern);
            if let Some(p) = paint(&texture, banner::dye_rgb(&layer.color)) {
                draw_part(flag, "root", m, &wave, p, out);
            }
        }
        true
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
