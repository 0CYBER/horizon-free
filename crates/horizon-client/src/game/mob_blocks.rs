//! Blocks mobs carry: block items in a hand (`ItemInHandLayer`) and the
//! block an enderman holds (`CarriedBlockLayer`), drawn as shaded cubes like
//! dropped block items.
use super::{Game, Icon};
use horizon_client::{
    mesher::Vertex,
    mobs::{Mob, Species, render},
};

/// The six faces of the unit cube with their shade: top, bottom, the two
/// z faces, the two x faces.
const FACES: [([[f32; 3]; 4], f32, u8); 6] = [
    (
        [[0., 1., 1.], [1., 1., 1.], [1., 1., 0.], [0., 1., 0.]],
        1.0,
        0,
    ),
    (
        [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]],
        0.5,
        0,
    ),
    (
        [[1., 0., 0.], [0., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
        0.8,
        1,
    ),
    (
        [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
        0.8,
        1,
    ),
    (
        [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]],
        0.6,
        2,
    ),
    (
        [[1., 0., 1.], [1., 0., 0.], [1., 1., 0.], [1., 1., 1.]],
        0.6,
        2,
    ),
];
/// The sprites of a cube's top, z sides and x sides, and the top's tint.
struct Cube<'a> {
    sprites: [&'a str; 3],
    tint: u32,
}

/// `CustomHeadLayer.translateToHead` and the banner's `head` display.
fn banner_matrix(head: glam::Mat4) -> glam::Mat4 {
    head * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.25, 0.0))
        * glam::Mat4::from_rotation_y(std::f32::consts::PI)
        * glam::Mat4::from_scale(glam::Vec3::new(0.625, -0.625, -0.625))
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, 1.0, 7.0 / 16.0))
        * glam::Mat4::from_rotation_y(std::f32::consts::PI)
        * glam::Mat4::from_scale(glam::Vec3::splat(1.5))
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.5, 0.0))
        * glam::Mat4::from_scale(glam::Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0))
}

/// `ElderGuardianParticle.lifetime`.
pub(super) const APPARITION_TICKS: i32 = 30;

impl Game<'_> {
    /// Count the elder guardian's apparition down.
    pub(super) fn tick_apparition(&mut self) {
        self.creatures.elder_apparition = (self.creatures.elder_apparition - 1).max(0);
    }
    /// `ElderGuardianParticleGroup`: the ghostly elder guardian that swings
    /// up in front of the camera while the curse is laid, fading in and out.
    pub(super) fn elder_apparition_geometry(&self, alpha: f64, out: &mut Vec<Vertex>) {
        let left = self.creatures.elder_apparition;
        if left <= 0 {
            return;
        }
        let Some(kind) = self.res.mob_types.get("minecraft:elder_guardian") else {
            return;
        };
        let Some(r) = self.res.atlas.sprites.get("entity/guardian_elder").copied() else {
            return;
        };
        let age = (APPARITION_TICKS - left) as f32 + alpha as f32;
        let t = (age / APPARITION_TICKS as f32).clamp(0.0, 1.0);
        let fade = 0.05 + 0.5 * (t * std::f32::consts::PI).sin();
        let color = (((fade * 255.0) as u32) << 24) | 0x00FF_FFFF;
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let forward = glam::Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        // The camera's rotation: local +x right, +y up, +z behind.
        let camera = glam::Mat4::from_cols(
            right.extend(0.0),
            up.extend(0.0),
            (-forward).extend(0.0),
            glam::Vec4::W,
        );
        let k = 0.425_531_92;
        let m = camera
            * glam::Mat4::from_rotation_x((60.0 - 150.0 * t).to_radians())
            * glam::Mat4::from_scale(glam::Vec3::new(k, -k, -k))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.56, 3.5));
        let poses = horizon_client::mobs::anim::Poses::new(&kind.model);
        render::draw(
            &kind.model,
            m,
            &poses,
            [r.u0, r.v0, r.u1, r.v1],
            color,
            240 | 240 << 8,
            0,
            out,
        );
    }
    /// The ominous banner a raid captain wears on its head: the banner model
    /// with its eight pattern layers (`CustomHeadLayer`).
    fn captain_banner_geometry(
        &self,
        mob: &Mob,
        camera: glam::DVec3,
        alpha: f64,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        use horizon_client::mobs::{Part, anim::Poses, load_model};
        static MODELS: std::sync::OnceLock<Option<(Part, Part)>> = std::sync::OnceLock::new();
        if !horizon_client::mobs::raider::is_captain(mob) {
            return;
        }
        let Some((pole, flag)) = MODELS.get_or_init(|| {
            let root = &self.options.resources;
            Some((
                load_model(root, "minecraft:standing_banner#main")?,
                load_model(root, "minecraft:standing_banner#flag")?,
            ))
        }) else {
            return;
        };
        let Some(head) =
            render::part_transform(mob, alpha as f32, camera, &self.res.mob_data, "head")
        else {
            return;
        };
        let m = banner_matrix(head);
        let atlas = &self.res.atlas;
        let uv = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // BannerRenderer: the flag sways with a per-banner phase.
        let phase =
            (((u64::from(mob.id) * 7 + u64::from(mob.age)) % 100) as f32 + alpha as f32) / 100.0;
        let mut flag_poses = Poses::new(flag);
        flag_poses.xrot(
            "flag",
            (-0.0125 + 0.01 * (2.0 * std::f32::consts::PI * phase).cos()) * std::f32::consts::PI,
        );
        let still = Poses::new(pole);
        let white = 0xFFFF_FFFF;
        if let Some(r) = uv("entity/banner/banner_base") {
            render::draw(pole, m, &still, r, white, light, 0, out);
            render::draw(flag, m, &flag_poses, r, white, light, 0, out);
        }
        let layers =
            std::iter::once(("base", "white")).chain(horizon_client::item_data::OMINOUS_LAYERS);
        for (pattern, dye) in layers {
            let Some(r) = uv(&format!("entity/banner/{pattern}")) else {
                continue;
            };
            let index = render::DYE_NAMES
                .iter()
                .position(|n| *n == dye)
                .unwrap_or(0);
            let color = render::abgr(render::DYE[index]);
            render::draw(flag, m, &flag_poses, r, color, light, 0, out);
        }
    }
    /// Resolve the icons of the items mobs hold, so drawing can look them up.
    pub(super) fn cache_mob_icons(&mut self) {
        let items: Vec<String> = self
            .mobs
            .iter()
            .filter(|m| m.extra.main_hand.is_some() || m.extra.off_hand.is_some())
            .flat_map(|m| [m.extra.main_hand.clone(), m.extra.off_hand.clone()])
            .flatten()
            .map(|i| i.to_string())
            .collect();
        for item in items {
            if !self.res.icons.contains_key(&item) {
                let _ = self.icon(&item);
            }
        }
    }
    fn push_cube(
        &self,
        t: glam::Mat4,
        cube: &Cube,
        light: u32,
        overlay: u32,
        out: &mut Vec<Vertex>,
    ) {
        for (corners, shade, side) in FACES {
            let name = cube.sprites[usize::from(side)];
            let Some(r) = self.res.atlas.sprites.get(name).copied() else {
                continue;
            };
            let tint = if side == 0 { cube.tint } else { 0xFF_FFFF };
            let c = |sft: u32| (((tint >> sft) & 255) as f32 * shade) as u32;
            let color = 0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16);
            let uvs = [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
            for (corner, uv) in corners.iter().zip(uvs) {
                let p = t.transform_point3(glam::Vec3::from(*corner));
                out.push(Vertex {
                    pos: p.to_array(),
                    uv,
                    layer: overlay,
                    color,
                    light,
                });
            }
        }
    }
    /// The blocks one mob holds or carries.
    pub(super) fn mob_block_geometry(
        &self,
        mob: &Mob,
        camera: glam::DVec3,
        alpha: f64,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let overlay = u32::from(mob.hurt_time > 0);
        self.captain_banner_geometry(mob, camera, alpha, light, out);
        for (item, left) in [(&mob.extra.main_hand, false), (&mob.extra.off_hand, true)] {
            let Some(item) = item else { continue };
            let Some(Icon::Cube {
                top,
                left: z,
                right: x,
                tint,
            }) = self.res.icons.get(&**item).cloned().flatten()
            else {
                continue;
            };
            let Some(t) =
                render::held_block_transform(mob, alpha as f32, camera, &self.res.mob_data, left)
            else {
                continue;
            };
            let slot = if left {
                horizon_client::mobs::gear::OFF
            } else {
                horizon_client::mobs::gear::MAIN
            };
            let glint = horizon_client::mobs::gear::enchanted(mob, slot);
            let overlay = if glint { 7 } else { overlay };
            self.push_cube(
                t,
                &Cube {
                    sprites: [&top, &z, &x],
                    tint,
                },
                light,
                overlay,
                out,
            );
        }
        if mob.species() == Species::Enderman
            && let Some(state) = mob.extra.carried
            && let Some((sprite, tint)) = self.particle_sprite(state)
        {
            // CarriedBlockLayer.
            let base = render::base_transform(mob, alpha as f32, camera);
            let t = base
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.6875, -0.75))
                * glam::Mat4::from_rotation_x(20.0f32.to_radians())
                * glam::Mat4::from_rotation_y(45.0f32.to_radians())
                * glam::Mat4::from_translation(glam::Vec3::new(0.25, 0.1875, 0.25))
                * glam::Mat4::from_scale(glam::Vec3::new(-0.5, -0.5, 0.5))
                * glam::Mat4::from_rotation_y(90.0f32.to_radians())
                * glam::Mat4::from_translation(glam::Vec3::splat(-0.5));
            self.push_cube(
                t,
                &Cube {
                    sprites: [&sprite, &sprite, &sprite],
                    tint,
                },
                light,
                overlay,
                out,
            );
        }
    }
}
