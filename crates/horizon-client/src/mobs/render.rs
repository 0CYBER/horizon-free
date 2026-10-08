//! Mob rendering (`LivingEntityRenderer.submit` and the render layers):
//! variant/baby model and texture selection, setupRotations, per-species
//! scale, and extra layers (sheep wool, spider eyes, drowned/stray outer
//! layers, villager overlays, collars...).
use super::{
    Mob, MobData, Part, Species,
    anim::{self, Poses},
    living::wrap,
};
use crate::mesher::Vertex;
use std::f32::consts::PI;

/// A texture lookup into the shared atlas (name without namespace -> uv rect).
pub type Atlas<'a> = &'a dyn Fn(&str) -> Option<[f32; 4]>;

/// One draw of a model with a texture.
pub struct Layer {
    pub model: String,
    pub texture: String,
    pub color: u32,
    pub full_bright: bool,
}
/// DyeColor.getTextureDiffuseColor (RGB).
pub const DYE: [u32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
    0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];
pub const DYE_NAMES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
/// ABGR vertex colour from an RGB value.
pub fn abgr(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    0xFF00_0000 | b << 16 | g << 8 | r
}
/// ColorLerper.Type.SHEEP colour: white is 0xE6E6E6, others 75% of the dye.
pub fn sheep_wool_rgb(color: u8) -> u32 {
    if color == 0 {
        return 0xE6E6E6;
    }
    let src = DYE[color as usize & 15];
    let f = |c: u32| ((c as f32 * 0.75).floor() as u32).min(255);
    f(src >> 16 & 255) << 16 | f(src >> 8 & 255) << 8 | f(src & 255)
}
const VILLAGER_TYPES: [&str; 7] = [
    "plains", "desert", "jungle", "savanna", "snow", "swamp", "taiga",
];
pub const PROFESSIONS: [&str; 15] = [
    "none",
    "armorer",
    "butcher",
    "cartographer",
    "cleric",
    "farmer",
    "fisherman",
    "fletcher",
    "leatherworker",
    "librarian",
    "mason",
    "nitwit",
    "shepherd",
    "toolsmith",
    "weaponsmith",
];
fn baby_or(m: &Mob, adult: &str, baby: &str) -> String {
    if m.is_baby() && m.kind.layer(baby).is_some() {
        baby.to_owned()
    } else {
        adult.to_owned()
    }
}
fn climate(m: &Mob) -> &'static str {
    match m.extra.variant {
        1 => "warm",
        2 => "cold",
        _ => "temperate",
    }
}
/// The model layer key and texture for the mob's body.
pub fn body(m: &Mob) -> (String, String) {
    use Species as S;
    let baby = m.is_baby();
    let b = |s: &str| {
        if baby {
            format!("{s}_baby")
        } else {
            s.to_owned()
        }
    };
    match m.species() {
        S::Pig => {
            let model = if baby {
                "pig_baby#main".into()
            } else if m.extra.variant == 2 {
                "cold_pig#main".into()
            } else {
                "pig#main".into()
            };
            (model, b(&format!("entity/pig/pig_{}", climate(m))))
        }
        S::Cow => {
            let model = match (m.extra.variant, baby) {
                (1, false) => "warm_cow#main",
                (1, true) => "warm_cow_baby#main",
                (2, false) => "cold_cow#main",
                (2, true) => "cold_cow_baby#main",
                (_, true) => "cow_baby#main",
                _ => "cow#main",
            };
            (model.into(), b(&format!("entity/cow/cow_{}", climate(m))))
        }
        S::Mooshroom => (
            baby_or(m, "mooshroom#main", "mooshroom_baby#main"),
            b(if m.extra.variant == 1 {
                "entity/cow/mooshroom_brown"
            } else {
                "entity/cow/mooshroom_red"
            }),
        ),
        S::Chicken => {
            let model = if baby {
                "chicken_baby#main"
            } else if m.extra.variant == 2 {
                "cold_chicken#main"
            } else {
                "chicken#main"
            };
            (
                model.into(),
                b(&format!("entity/chicken/chicken_{}", climate(m))),
            )
        }
        S::Sheep => (
            baby_or(m, "sheep#main", "sheep_baby#main"),
            b("entity/sheep/sheep"),
        ),
        S::Zombie => (
            baby_or(m, "zombie#main", "zombie_baby#main"),
            b("entity/zombie/zombie"),
        ),
        S::Husk => (
            baby_or(m, "husk#main", "husk_baby#main"),
            b("entity/zombie/husk"),
        ),
        S::Drowned => (
            baby_or(m, "drowned#main", "drowned_baby#main"),
            b("entity/zombie/drowned"),
        ),
        S::ZombieVillager => (
            baby_or(m, "zombie_villager#main", "zombie_villager_baby#main"),
            "entity/zombie_villager/zombie_villager".into(),
        ),
        S::Villager => (
            baby_or(m, "villager#main", "villager_baby#main"),
            b("entity/villager/villager"),
        ),
        S::Piglin => (
            baby_or(m, "piglin#main", "piglin_baby#main"),
            b("entity/piglin/piglin"),
        ),
        S::ZombifiedPiglin => (
            baby_or(m, "zombified_piglin#main", "zombified_piglin_baby#main"),
            b("entity/piglin/zombified_piglin"),
        ),
        S::Hoglin => (
            baby_or(m, "hoglin#main", "hoglin_baby#main"),
            b("entity/hoglin/hoglin"),
        ),
        S::Zoglin => (
            baby_or(m, "zoglin#main", "zoglin_baby#main"),
            b("entity/hoglin/zoglin"),
        ),
        S::Strider => (
            baby_or(m, "strider#main", "strider_baby#main"),
            b(if m.extra.flag {
                "entity/strider/strider_cold"
            } else {
                "entity/strider/strider"
            }),
        ),
        _ => (m.kind.model_key(), m.kind.texture.clone()),
    }
}
/// Extra render layers drawn over the body with the same pose.
pub fn layers(m: &Mob) -> Vec<Layer> {
    use Species as S;
    let mut v = Vec::new();
    let white = 0xFFFF_FFFF;
    let push = |v: &mut Vec<Layer>, model: &str, texture: &str, color: u32, full_bright: bool| {
        v.push(Layer {
            model: model.into(),
            texture: texture.into(),
            color,
            full_bright,
        })
    };
    match m.species() {
        S::Sheep if !m.extra.sheared => {
            let color = abgr(sheep_wool_rgb(m.extra.color));
            if !m.is_baby() && m.extra.color != 0 {
                push(
                    &mut v,
                    "sheep#wool_undercoat",
                    "entity/sheep/sheep_wool_undercoat",
                    color,
                    false,
                );
            }
            if m.is_baby() {
                push(
                    &mut v,
                    "sheep_baby#wool",
                    "entity/sheep/sheep_wool_baby",
                    color,
                    false,
                );
            } else {
                push(
                    &mut v,
                    "sheep#wool",
                    "entity/sheep/sheep_wool",
                    color,
                    false,
                );
            }
        }
        S::Spider | S::CaveSpider => push(
            &mut v,
            &m.kind.model_key(),
            "entity/spider/spider_eyes",
            white,
            true,
        ),
        S::Enderman => push(
            &mut v,
            "enderman#main",
            "entity/enderman/enderman_eyes",
            white,
            true,
        ),
        S::Phantom => push(
            &mut v,
            "phantom#main",
            "entity/phantom/phantom_eyes",
            white,
            true,
        ),
        S::Drowned => {
            if m.is_baby() {
                push(
                    &mut v,
                    "drowned_baby#outer",
                    "entity/zombie/drowned_outer_layer_baby",
                    white,
                    false,
                );
            } else {
                push(
                    &mut v,
                    "drowned#outer",
                    "entity/zombie/drowned_outer_layer",
                    white,
                    false,
                );
            }
        }
        S::Stray => push(
            &mut v,
            "stray#outer",
            "entity/skeleton/stray_overlay",
            white,
            false,
        ),
        S::Bogged if !m.extra.sheared => push(
            &mut v,
            "bogged#outer",
            "entity/skeleton/bogged_overlay",
            white,
            false,
        ),
        S::Creeper if m.extra.charged => {}
        S::Villager | S::ZombieVillager => {
            let base = if m.species() == S::Villager {
                "villager"
            } else {
                "zombie_villager"
            };
            let model = if m.is_baby() {
                format!("{base}_baby#main")
            } else {
                format!("{base}#main")
            };
            let t = VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()];
            let folder = if m.species() == S::Villager {
                "entity/villager"
            } else {
                "entity/zombie_villager"
            };
            let baby = if m.is_baby() && m.species() == S::Villager {
                "baby/"
            } else {
                ""
            };
            push(
                &mut v,
                &model,
                &format!("{folder}/type/{baby}{t}"),
                white,
                false,
            );
            let prof = PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()];
            if !m.is_baby() && prof != "none" {
                push(
                    &mut v,
                    &model,
                    &format!("{folder}/profession/{prof}"),
                    white,
                    false,
                );
                if m.species() == S::Villager && prof != "nitwit" {
                    let level = ["stone", "iron", "gold", "emerald", "diamond"]
                        [(m.extra.variant3 as usize).min(4)];
                    push(
                        &mut v,
                        &model,
                        &format!("{folder}/profession_level/{level}"),
                        white,
                        false,
                    );
                }
            }
        }
        _ => {}
    }
    v
}
impl super::MobType {
    /// The main model layer key without the namespace.
    pub fn model_key(&self) -> String {
        super::species::def(self.species).model.to_owned()
    }
}
/// The mob's root transform (LivingEntityRenderer.submit + setupRotations).
pub fn base_transform(m: &Mob, alpha: f32, camera: glam::DVec3) -> glam::Mat4 {
    let a = f64::from(alpha);
    let p = glam::DVec3::new(
        m.previous.x + (m.position.x - m.previous.x) * a,
        m.previous.y + (m.position.y - m.previous.y) * a,
        m.previous.z + (m.position.z - m.previous.z) * a,
    ) - camera;
    let body = m.previous_body_yaw + wrap(m.body_yaw - m.previous_body_yaw) * alpha;
    let mut base = glam::Mat4::from_translation(p.as_vec3())
        * glam::Mat4::from_rotation_y((180.0 - body).to_radians());
    if m.death_time > 0 {
        let f = ((m.death_time as f32 + alpha - 1.0) / 20.0 * 1.6)
            .sqrt()
            .min(1.0);
        base *= glam::Mat4::from_rotation_z(f * flip_degrees(m).to_radians());
    }
    base *= species_rotations(m, alpha);
    base *= glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
    base *= species_scale(m, alpha);
    base * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0))
}
fn flip_degrees(m: &Mob) -> f32 {
    match m.species() {
        Species::Spider | Species::CaveSpider | Species::Silverfish | Species::Endermite => 180.0,
        _ => 90.0,
    }
}
fn species_rotations(_m: &Mob, _alpha: f32) -> glam::Mat4 {
    let _ = PI;
    glam::Mat4::IDENTITY
}
fn species_scale(m: &Mob, alpha: f32) -> glam::Mat4 {
    match m.species() {
        Species::Creeper => {
            // CreeperRenderer.scale: swelling with a wobble.
            let s = ((m.previous_swell as f32 + (m.swell - m.previous_swell) as f32 * alpha)
                / 28.0)
                .clamp(0.0, 1.0);
            let wobble = 1.0 + (s * 100.0).sin() * s * 0.01;
            let s4 = s * s * s * s;
            let xz = (1.0 + s4 * 0.4) * wobble;
            let y = (1.0 + s4 * 0.1) / wobble;
            glam::Mat4::from_scale(glam::Vec3::new(xz, y, xz))
        }
        _ => glam::Mat4::IDENTITY,
    }
}
/// Draw a part tree with a pose map, texture rect, colour and light.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    part: &Part,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    let p = poses.pose(part);
    let m = base
        * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
    if p[6] == 0.0 && p[7] == 0.0 && p[8] == 0.0 {
        return;
    }
    for poly in &part.polygons {
        for v in poly.vertices {
            let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
            out.push(Vertex {
                pos: pos.to_array(),
                uv: [
                    uv[0] + (uv[2] - uv[0]) * v[3],
                    uv[1] + (uv[3] - uv[1]) * v[4],
                ],
                layer: overlay,
                color,
                light,
            });
        }
    }
    for c in &part.children {
        draw(c, m, poses, uv, color, light, overlay, out);
    }
}
/// Render a mob with its variant model, texture and layers.
pub fn render(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform(m, alpha, camera);
    let overlay = u32::from(m.hurt_time > 0 || m.death_time > 0);
    let s = anim::state(m, alpha);
    let (model_key, texture) = body(m);
    let model = m
        .kind
        .layer(&model_key)
        .cloned()
        .unwrap_or_else(|| m.kind.model.clone());
    if let Some(uv) = atlas(&texture) {
        let mut poses = Poses::new(&model);
        anim::setup(m, &s, &mut poses, &data.animations);
        draw(&model, base, &poses, uv, 0xFFFF_FFFF, light, overlay, out);
    }
    for layer in layers(m) {
        let Some(part) = m.kind.layer(&layer.model).cloned() else {
            continue;
        };
        let Some(uv) = atlas(&layer.texture) else {
            continue;
        };
        let mut poses = Poses::new(&part);
        anim::setup(m, &s, &mut poses, &data.animations);
        let light = if layer.full_bright {
            240 | 240 << 8
        } else {
            light
        };
        draw(&part, base, &poses, uv, layer.color, light, overlay, out);
    }
}
/// Render with a fixed texture rect (the earlier API).
pub fn render_single(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform(m, alpha, camera);
    let overlay = u32::from(m.hurt_time > 0 || m.death_time > 0);
    let s = anim::state(m, alpha);
    let model = m.kind.model.clone();
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &std::collections::HashMap::new());
    draw(&model, base, &poses, uv, 0xFFFF_FFFF, light, overlay, out);
}
