//! Block-entity renderers (`BlockEntityRenderers`): chests (single, double,
//! trapped, ender, copper), shulker boxes, banners, skulls and heads, bells,
//! decorated pots, conduits, enchanting-table and lectern books and copper
//! golem statues. Models are vanilla's baked `LayerDefinitions` layers
//! (`entity-models.json`); transforms, animation and textures follow each
//! vanilla renderer. Faces are lit like the `entity` shader: the lightmap at
//! the block plus `minecraft_mix_light` with the level's two diffuse lights.
use crate::{hud::Rect, mesher::Vertex, world::World};
use glam::{DVec3, Mat4, Quat, Vec3};
use horizon_core::{
    geometry::trig::Mth,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use serde_json::Value;
use std::{collections::HashMap, f32::consts::PI, path::Path};

/// One baked `ModelPart`: initial pose, textured polygons (pixel units,
/// normalized UVs) and named children.
#[derive(Clone, Debug, Default)]
pub struct ModelPart {
    /// x, y, z, xRot, yRot, zRot, xScale, yScale, zScale.
    pub pose: [f32; 9],
    pub polygons: Vec<[[f32; 5]; 4]>,
    pub children: Vec<(String, ModelPart)>,
}
impl ModelPart {
    pub fn parse(v: &Value) -> Self {
        let mut pose = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        if let Some(a) = v["pose"].as_array() {
            for (p, x) in pose.iter_mut().zip(a) {
                *p = x.as_f64().unwrap_or(0.0) as f32;
            }
        }
        let polygons = v["polygons"]
            .as_array()
            .map(|polys| {
                polys
                    .iter()
                    .filter_map(|poly| {
                        let verts = poly.as_array()?;
                        if verts.len() != 4 {
                            return None;
                        }
                        let mut out = [[0f32; 5]; 4];
                        for (o, vert) in out.iter_mut().zip(verts) {
                            for (c, x) in o.iter_mut().zip(vert.as_array()?) {
                                *c = x.as_f64().unwrap_or(0.0) as f32;
                            }
                        }
                        Some(out)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let children = v["children"]
            .as_object()
            .map(|m| m.iter().map(|(k, c)| (k.clone(), Self::parse(c))).collect())
            .unwrap_or_default();
        Self {
            pose,
            polygons,
            children,
        }
    }
    pub fn child(&self, name: &str) -> Option<&ModelPart> {
        self.children
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c)
    }
}

/// `Lighting` level light directions (overworld/end and nether).
pub fn diffuse_lights(nether: bool) -> (Vec3, Vec3) {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = if nether {
        Vec3::new(-0.2, -1.0, 0.7)
    } else {
        Vec3::new(-0.2, 1.0, 0.7)
    }
    .normalize();
    (l0, l1)
}
/// `minecraft_mix_light`: 0.6 power, 0.4 ambient.
pub fn diffuse(normal: Vec3, lights: (Vec3, Vec3)) -> f32 {
    ((lights.0.dot(normal).max(0.0) + lights.1.dot(normal).max(0.0)) * 0.6 + 0.4).min(1.0)
}
/// Packed vertex light from the level's packed `sky << 4 | block` byte.
pub fn vertex_light(packed: u8) -> u32 {
    (u32::from(packed >> 4) * 16) | ((u32::from(packed & 15) * 16) << 8)
}
fn pack_color(rgb: u32, alpha: u32, brightness: f32) -> u32 {
    let c = |shift: u32| (((rgb >> shift) & 255) as f32 * brightness) as u32;
    alpha << 24 | c(0) << 16 | c(8) << 8 | c(16)
}

/// Everything a draw call needs besides the geometry.
#[derive(Clone, Copy)]
pub struct Paint {
    pub sprite: Rect,
    /// 0xRRGGBB multiplied with the texture (`-1` = white in vanilla).
    pub tint: u32,
    pub light: u32,
    pub lights: (Vec3, Vec3),
}
/// `ModelPart.render`: translate by pose/16, rotate ZYX, scale; children inherit.
/// `adjust` may change a part's pose by name (`setupAnim`); `skip` hides parts.
pub fn draw_part(
    part: &ModelPart,
    name: &str,
    parent: Mat4,
    adjust: &dyn Fn(&str, &mut [f32; 9]),
    paint: Paint,
    out: &mut Vec<Vertex>,
) {
    let mut p = part.pose;
    adjust(name, &mut p);
    let m = parent
        * Mat4::from_translation(Vec3::new(p[0], p[1], p[2]) / 16.0)
        * Mat4::from_quat(Quat::from_euler(glam::EulerRot::ZYX, p[5], p[4], p[3]))
        * Mat4::from_scale(Vec3::new(p[6], p[7], p[8]));
    let Paint {
        sprite: r,
        tint,
        light,
        lights,
    } = paint;
    for poly in &part.polygons {
        let pos = poly.map(|v| m.transform_point3(Vec3::new(v[0], v[1], v[2]) / 16.0));
        let normal = (pos[1] - pos[0]).cross(pos[2] - pos[0]).normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            // Degenerate polygon: use the other diagonal.
            (pos[2] - pos[1])
                .cross(pos[3] - pos[1])
                .normalize_or(Vec3::Y)
        } else {
            normal
        };
        let color = pack_color(tint, 255, diffuse(normal, lights));
        for (v, position) in poly.iter().zip(pos) {
            out.push(Vertex {
                pos: position.to_array(),
                uv: [r.u0 + (r.u1 - r.u0) * v[3], r.v0 + (r.v1 - r.v0) * v[4]],
                layer: 0,
                color,
                light,
            });
        }
    }
    for (child_name, child) in &part.children {
        draw_part(child, child_name, m, adjust, paint, out);
    }
}

/// `Direction.toYRot` for horizontal facings (south 0, west 90, north 180, east 270).
fn y_rot(facing: &str) -> f32 {
    match facing {
        "west" => 90.0,
        "north" => 180.0,
        "east" => 270.0,
        _ => 0.0,
    }
}
fn opposite(facing: &str) -> &'static str {
    match facing {
        "north" => "south",
        "south" => "north",
        "west" => "east",
        "east" => "west",
        "up" => "down",
        _ => "up",
    }
}
fn clockwise(facing: &str) -> &'static str {
    match facing {
        "north" => "east",
        "east" => "south",
        "south" => "west",
        _ => "north",
    }
}
fn step(facing: &str) -> [i32; 3] {
    match facing {
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        "east" => [1, 0, 0],
        "up" => [0, 1, 0],
        _ => [0, -1, 0],
    }
}
/// `Direction.getRotation`.
fn direction_rotation(facing: &str) -> Quat {
    let xyz = |x: f32, y: f32, z: f32| Quat::from_euler(glam::EulerRot::XYZ, x, y, z);
    match facing {
        "down" => Quat::from_rotation_x(PI),
        "north" => xyz(PI / 2.0, 0.0, PI),
        "south" => Quat::from_rotation_x(PI / 2.0),
        "west" => xyz(PI / 2.0, 0.0, PI / 2.0),
        "east" => xyz(PI / 2.0, 0.0, -PI / 2.0),
        _ => Quat::IDENTITY,
    }
}
fn around(rotation: Mat4, pivot: Vec3) -> Mat4 {
    Mat4::from_translation(pivot) * rotation * Mat4::from_translation(-pivot)
}

/// `DyeColor` names and texture diffuse colours, by id.
pub const DYES: [(&str, u32); 16] = [
    ("white", 0xF9FFFE),
    ("orange", 0xF9801D),
    ("magenta", 0xC74EBD),
    ("light_blue", 0x3AB3DA),
    ("yellow", 0xFED83D),
    ("lime", 0x80C71F),
    ("pink", 0xF38BAA),
    ("gray", 0x474F52),
    ("light_gray", 0x9D9D97),
    ("cyan", 0x169C9C),
    ("purple", 0x8932B8),
    ("blue", 0x3C44AA),
    ("brown", 0x835432),
    ("green", 0x5E7C16),
    ("red", 0xB02E26),
    ("black", 0x1D1D21),
];
fn dye_prefix(name: &str) -> Option<(usize, &str)> {
    // Longest names first so "light_gray" wins over "gray".
    let mut order: Vec<usize> = (0..16).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(DYES[i].0.len()));
    order.into_iter().find_map(|i| {
        name.strip_prefix(DYES[i].0)
            .and_then(|rest| rest.strip_prefix('_'))
            .map(|rest| (i, rest))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChestPart {
    Single,
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkullType {
    Skeleton,
    WitherSkeleton,
    Zombie,
    Creeper,
    Dragon,
    Piglin,
    Player,
}
#[derive(Clone, Debug)]
enum Kind {
    None,
    Chest {
        texture: &'static str,
        part: ChestPart,
        y_rot: f32,
        double_with: Option<[i32; 3]>,
    },
    Shulker {
        texture: String,
        facing: &'static str,
    },
    Banner {
        color: u32,
        wall: bool,
        angle: f32,
    },
    Skull {
        kind: SkullType,
        wall: Option<&'static str>,
        segment: u8,
    },
    Bell,
    DecoratedPot {
        y_rot: f32,
    },
    Conduit,
    EnchantingTable,
    Lectern {
        y_rot: f32,
    },
    CopperGolem {
        texture: &'static str,
        pose: &'static str,
        facing: &'static str,
    },
    /// Sign text (`StandingSignRenderer` / `HangingSignRenderer`); the boards are block models.
    Sign {
        hanging: bool,
        wall: bool,
        angle: f32,
    },
}
fn intern(facing: &str) -> &'static str {
    match facing {
        "north" => "north",
        "south" => "south",
        "west" => "west",
        "east" => "east",
        "up" => "up",
        _ => "down",
    }
}
fn classify(name: &str, prop: &dyn Fn(&str) -> Option<String>) -> Kind {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let facing = || intern(&prop("facing").unwrap_or_default());
    let chest = |texture: &'static str, left: &'static str, right: &'static str| {
        let facing = facing();
        let part = match prop("type").as_deref() {
            Some("left") => ChestPart::Left,
            Some("right") => ChestPart::Right,
            _ => ChestPart::Single,
        };
        // ChestBlock.getConnectedDirection: left connects clockwise, right counter-clockwise.
        let other = match part {
            ChestPart::Single => None,
            ChestPart::Left => Some(step(clockwise(facing))),
            ChestPart::Right => Some(step(opposite(clockwise(facing)))),
        };
        Kind::Chest {
            texture: match part {
                ChestPart::Single => texture,
                ChestPart::Left => left,
                ChestPart::Right => right,
            },
            part,
            y_rot: y_rot(facing),
            double_with: other,
        }
    };
    if let Some(hanging) = crate::signs::sign_kind(name) {
        let wall = name.contains("_wall_");
        let angle = if wall {
            y_rot(facing())
        } else {
            prop("rotation")
                .and_then(|r| r.parse::<f32>().ok())
                .unwrap_or(0.0)
                * 22.5
        };
        return Kind::Sign {
            hanging,
            wall,
            angle,
        };
    }
    let unwaxed = name.strip_prefix("waxed_").unwrap_or(name);
    match unwaxed {
        "chest" => {
            return chest(
                "entity/chest/normal",
                "entity/chest/normal_left",
                "entity/chest/normal_right",
            );
        }
        "trapped_chest" => {
            return chest(
                "entity/chest/trapped",
                "entity/chest/trapped_left",
                "entity/chest/trapped_right",
            );
        }
        "ender_chest" => {
            return chest(
                "entity/chest/ender",
                "entity/chest/ender",
                "entity/chest/ender",
            );
        }
        "copper_chest" => {
            return chest(
                "entity/chest/copper",
                "entity/chest/copper_left",
                "entity/chest/copper_right",
            );
        }
        "exposed_copper_chest" => {
            return chest(
                "entity/chest/copper_exposed",
                "entity/chest/copper_exposed_left",
                "entity/chest/copper_exposed_right",
            );
        }
        "weathered_copper_chest" => {
            return chest(
                "entity/chest/copper_weathered",
                "entity/chest/copper_weathered_left",
                "entity/chest/copper_weathered_right",
            );
        }
        "oxidized_copper_chest" => {
            return chest(
                "entity/chest/copper_oxidized",
                "entity/chest/copper_oxidized_left",
                "entity/chest/copper_oxidized_right",
            );
        }
        "shulker_box" => {
            return Kind::Shulker {
                texture: "entity/shulker/shulker".into(),
                facing: facing(),
            };
        }
        "bell" => return Kind::Bell,
        "decorated_pot" => {
            return Kind::DecoratedPot {
                y_rot: y_rot(facing()),
            };
        }
        "conduit" => return Kind::Conduit,
        "enchanting_table" => return Kind::EnchantingTable,
        "lectern" => {
            return if prop("has_book").as_deref() == Some("true") {
                Kind::Lectern {
                    y_rot: y_rot(clockwise(facing())),
                }
            } else {
                Kind::None
            };
        }
        _ => {}
    }
    for (prefix, texture) in [
        ("copper_golem_statue", "entity/copper_golem/copper_golem"),
        (
            "exposed_copper_golem_statue",
            "entity/copper_golem/copper_golem_exposed",
        ),
        (
            "weathered_copper_golem_statue",
            "entity/copper_golem/copper_golem_weathered",
        ),
        (
            "oxidized_copper_golem_statue",
            "entity/copper_golem/copper_golem_oxidized",
        ),
    ] {
        if unwaxed == prefix {
            let pose = match prop("copper_golem_pose").as_deref() {
                Some("running") => "running",
                Some("sitting") => "sitting",
                Some("star") => "star",
                _ => "standing",
            };
            return Kind::CopperGolem {
                texture,
                pose,
                facing: facing(),
            };
        }
    }
    let skull = |base: &str| -> Option<SkullType> {
        Some(match base {
            "skeleton_skull" => SkullType::Skeleton,
            "wither_skeleton_skull" => SkullType::WitherSkeleton,
            "zombie_head" => SkullType::Zombie,
            "creeper_head" => SkullType::Creeper,
            "dragon_head" => SkullType::Dragon,
            "piglin_head" => SkullType::Piglin,
            "player_head" => SkullType::Player,
            _ => return None,
        })
    };
    if let Some(kind) = skull(name) {
        let segment = prop("rotation").and_then(|r| r.parse().ok()).unwrap_or(0);
        return Kind::Skull {
            kind,
            wall: None,
            segment,
        };
    }
    if name.contains("_wall_")
        && let Some(kind) = skull(&name.replace("_wall_", "_"))
    {
        return Kind::Skull {
            kind,
            wall: Some(facing()),
            segment: 0,
        };
    }
    if let Some((dye, rest)) = dye_prefix(name) {
        match rest {
            "shulker_box" => {
                return Kind::Shulker {
                    texture: format!("entity/shulker/shulker_{}", DYES[dye].0),
                    facing: facing(),
                };
            }
            "banner" => {
                let segment: f32 = prop("rotation").and_then(|r| r.parse().ok()).unwrap_or(0.0);
                return Kind::Banner {
                    color: DYES[dye].1,
                    wall: false,
                    angle: segment * 22.5,
                };
            }
            "wall_banner" => {
                return Kind::Banner {
                    color: DYES[dye].1,
                    wall: true,
                    angle: y_rot(facing()),
                };
            }
            _ => {}
        }
    }
    Kind::None
}

/// `EnchantingTableBlockEntity` book animation state.
#[derive(Clone, Copy, Default)]
struct Book {
    time: i32,
    flip: f32,
    o_flip: f32,
    flip_t: f32,
    flip_a: f32,
    open: f32,
    o_open: f32,
    rot: f32,
    o_rot: f32,
    t_rot: f32,
}
fn wrap(mut a: f32) -> f32 {
    while a >= PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}
impl Book {
    /// `EnchantingTableBlockEntity.bookAnimationTick`.
    fn tick(&mut self, pos: [i32; 3], player: Option<[f64; 3]>, random: &mut LegacyRandom) {
        self.o_open = self.open;
        self.o_rot = self.rot;
        let c = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let near = player.filter(|p| {
            (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2) < 9.0
        });
        if let Some(p) = near {
            self.t_rot = (p[2] - c[2]).atan2(p[0] - c[0]) as f32;
            self.open += 0.1;
            if self.open < 0.5 || random.next_int_bound(40).unwrap_or(1) == 0 {
                let old = self.flip_t;
                loop {
                    let a = random.next_int_bound(4).unwrap_or(0);
                    let b = random.next_int_bound(4).unwrap_or(0);
                    self.flip_t += (a - b) as f32;
                    if old != self.flip_t {
                        break;
                    }
                }
            }
        } else {
            self.t_rot += 0.02;
            self.open -= 0.1;
        }
        self.rot = wrap(self.rot);
        self.t_rot = wrap(self.t_rot);
        let dir = wrap(self.t_rot - self.rot);
        self.rot += dir * 0.4;
        self.open = self.open.clamp(0.0, 1.0);
        self.time += 1;
        self.o_flip = self.flip;
        let diff = ((self.flip_t - self.flip) * 0.4).clamp(-0.2, 0.2);
        self.flip_a += (diff - self.flip_a) * 0.9;
        self.flip += self.flip_a;
    }
}
/// `BookModel.setupAnim` for `BookModel.State(openness, pageFlip1, pageFlip2)`.
fn book_pose(openness: f32, flip1: f32, flip2: f32) -> impl Fn(&str, &mut [f32; 9]) {
    move |name, p| match name {
        "left_lid" => p[4] = PI + openness,
        "right_lid" => p[4] = -openness,
        "left_pages" => {
            p[4] = openness;
            p[0] = openness.msin();
        }
        "right_pages" => {
            p[4] = -openness;
            p[0] = openness.msin();
        }
        "flip_page1" => {
            p[4] = openness - openness * 2.0 * flip1;
            p[0] = openness.msin();
        }
        "flip_page2" => {
            p[4] = openness - openness * 2.0 * flip2;
            p[0] = openness.msin();
        }
        _ => {}
    }
}
fn frac(x: f32) -> f32 {
    x - x.floor()
}

/// `StringSplitter.splitLines(...).get(0)` for plain text: the first line,
/// wrapped at the last space that fits or mid-word when none does.
fn first_line(line: &str, max_width: f32, advance: &dyn Fn(char) -> f32) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut width = 0.0;
    let mut last_space = None;
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            return chars[..i].iter().collect();
        }
        if c == ' ' {
            last_space = Some(i);
        }
        width += advance(c);
        if width > max_width {
            let end = match last_space {
                Some(s) if s > 0 => s,
                _ => i.max(1),
            };
            return chars[..end].iter().collect();
        }
    }
    line.to_owned()
}

/// `AbstractSignRenderer.submitSignText`: four centred lines, glowing text
/// full-bright with an 8x outline in the dark colour.
fn sign_text_calls(
    scene: &Scene,
    data: &crate::signs::SignData,
    text: &crate::signs::SignText,
    m: Mat4,
    packed: u8,
    near: bool,
) -> Vec<TextCall> {
    let line_height = data.line_height();
    let midpoint = (4.0 * line_height / 2.0).floor();
    let dark = 0xFF00_0000 | text.dark_color();
    let (color, outline, light) = if text.glowing {
        let c = text.text_color();
        (
            0xFF00_0000 | c,
            if c == 0 || near { dark } else { 0 },
            240 | 240 << 8,
        )
    } else {
        (dark, 0, vertex_light(packed))
    };
    let advance = |c: char| (scene.glyph)(c).1;
    let width = |s: &str| s.chars().map(advance).sum::<f32>().ceil();
    text.lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let shown = first_line(line, data.max_line_width(), &advance);
            TextCall {
                matrix: m,
                x: -((width(&shown) as i32) / 2) as f32,
                y: i as f32 * line_height - midpoint,
                text: shown,
                color,
                outline,
                light,
            }
        })
        .collect()
}

/// Draws a text call with the font atlas: 8x8 glyph cells, the outline as
/// eight one-pixel offsets under the text.
fn draw_text(scene: &Scene, call: &TextCall, out: &mut Vec<Vertex>) {
    let mut emit = |dx: f32, dy: f32, argb: u32| {
        let mut x = call.x + dx;
        let y0 = call.y + dy;
        for ch in call.text.chars() {
            let (cell, advance) = (scene.glyph)(ch);
            if let Some(r) = cell {
                let corners = [(x, y0 + 8.0), (x + 8.0, y0 + 8.0), (x + 8.0, y0), (x, y0)];
                let uvs = [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
                let c =
                    0xFF00_0000 | (argb & 255) << 16 | (argb >> 8 & 255) << 8 | (argb >> 16 & 255);
                for ((cx, cy), uv) in corners.into_iter().zip(uvs) {
                    out.push(Vertex {
                        pos: call
                            .matrix
                            .transform_point3(Vec3::new(cx, cy, 0.0))
                            .to_array(),
                        uv,
                        layer: 0,
                        color: c,
                        light: call.light,
                    });
                }
            }
            x += advance;
        }
    };
    if call.outline != 0 {
        for (dx, dy) in [
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (-1.0, 0.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (0.0, 1.0),
            (1.0, 1.0),
        ] {
            emit(dx, dy, call.outline);
        }
    }
    emit(0.0, 0.0, call.color);
}

/// Block-entity fields the level ticks, at one position (vanilla's own field
/// values; extraction to render state happens here as in each renderer).
#[derive(Clone, Debug, Default)]
pub struct BlockAnim {
    /// `ShulkerBoxBlockEntity` progress and progressOld.
    pub shulker: (f32, f32),
    /// `BellBlockEntity.ticks` and the click direction while `shaking`.
    pub bell_ticks: i32,
    pub bell_shake: Option<&'static str>,
    /// `ConduitBlockEntity` tickCount, isActive, isHunting and activeRotation.
    pub conduit: (i32, bool, bool, f32),
    /// `SkullBlockEntity` isAnimating and animationTickCount.
    pub skull: (bool, i32),
    /// `DecoratedPotBlockEntity` decorations as item ids: back, left, right, front.
    pub sherds: [Option<String>; 4],
    /// `DecoratedPotRenderState` wobble style (positive) and progress.
    pub wobble: Option<(bool, f32)>,
}

/// One `submitText` call (`AbstractSignRenderer.submitSignText`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextCall {
    pub matrix: Mat4,
    pub x: f32,
    pub y: f32,
    pub text: String,
    /// ARGB.
    pub color: u32,
    /// ARGB outline colour, 0 for none.
    pub outline: u32,
    /// Horizon packed light (sky*16 | block*16 << 8).
    pub light: u32,
}

/// Inputs from the game for one frame of block-entity rendering.
pub struct Scene<'a> {
    pub camera: DVec3,
    /// `Camera.rotation` (conduit eyes face the camera).
    pub orientation: Quat,
    /// Ticked block-entity state at a position.
    pub anim: &'a dyn Fn([i32; 3]) -> BlockAnim,
    /// When set, sign text is recorded here as text calls instead of drawn.
    pub texts: Option<&'a std::cell::RefCell<Vec<TextCall>>>,
    pub partial: f32,
    pub game_time: i64,
    pub world: &'a World,
    pub sprites: &'a HashMap<String, Rect>,
    pub nether: bool,
    /// Chest lid openness (`ChestLidController.getOpenNess`) at a position.
    pub lid: &'a dyn Fn([i32; 3]) -> f32,
    /// Sign block-entity data at a position.
    pub sign: &'a dyn Fn([i32; 3]) -> Option<&'a crate::signs::SignData>,
    /// Banner layers at a position: pattern texture and RGB tint, bottom first.
    pub banner: &'a dyn Fn([i32; 3]) -> Vec<(String, u32)>,
    /// Font glyph cell (none for blank glyphs such as the space) and advance.
    pub glyph: &'a dyn Fn(char) -> (Option<Rect>, f32),
}

pub struct BlockEntityRenderer {
    models: HashMap<&'static str, ModelPart>,
    kinds: Vec<Kind>,
    books: HashMap<[i32; 3], Book>,
    random: LegacyRandom,
    /// Item id to its `PROVIDES_POTTERY_PATTERN` side texture (`Sheets.DECORATED_POT_MAPPER`).
    pot_patterns: HashMap<String, String>,
}
const MODELS: [(&str, &str); 27] = [
    ("chest", "minecraft:chest#main"),
    ("double_chest_left", "minecraft:double_chest_left#main"),
    ("double_chest_right", "minecraft:double_chest_right#main"),
    ("shulker_box", "minecraft:shulker_box#main"),
    ("standing_banner", "minecraft:standing_banner#main"),
    ("standing_banner_flag", "minecraft:standing_banner#flag"),
    ("wall_banner", "minecraft:wall_banner#main"),
    ("wall_banner_flag", "minecraft:wall_banner#flag"),
    ("skeleton_skull", "minecraft:skeleton_skull#main"),
    (
        "wither_skeleton_skull",
        "minecraft:wither_skeleton_skull#main",
    ),
    ("zombie_head", "minecraft:zombie_head#main"),
    ("creeper_head", "minecraft:creeper_head#main"),
    ("dragon_skull", "minecraft:dragon_skull#main"),
    ("piglin_head", "minecraft:piglin_head#main"),
    ("player_head", "minecraft:player_head#main"),
    ("bell", "minecraft:bell#main"),
    ("book", "minecraft:book#main"),
    ("conduit_shell", "minecraft:conduit#shell"),
    ("conduit_cage", "minecraft:conduit#cage"),
    ("conduit_wind", "minecraft:conduit#wind"),
    ("conduit_eye", "minecraft:conduit#eye"),
    ("decorated_pot_base", "minecraft:decorated_pot_base#main"),
    ("decorated_pot_sides", "minecraft:decorated_pot_sides#main"),
    ("copper_golem", "minecraft:copper_golem#main"),
    (
        "copper_golem_running",
        "minecraft:copper_golem_running#main",
    ),
    (
        "copper_golem_sitting",
        "minecraft:copper_golem_sitting#main",
    ),
    ("copper_golem_star", "minecraft:copper_golem_star#main"),
];
/// `BlockEntityRenderer.getViewDistance` default.
const VIEW_DISTANCE: f64 = 64.0;

impl BlockEntityRenderer {
    pub fn load(root: &Path, registry: &BlockRegistry) -> crate::Result<Self> {
        let all: Value = serde_json::from_slice(&std::fs::read(root.join("entity-models.json"))?)?;
        let models = MODELS
            .iter()
            .filter_map(|(key, layer)| all.get(*layer).map(|v| (*key, ModelPart::parse(v))))
            .collect();
        let mut renderer = Self::new(models, registry);
        // Sherd items: provides_pottery_pattern -> decorated_pot_pattern asset id.
        if let Ok(bytes) = std::fs::read(root.join("items.json")) {
            let items: Value = serde_json::from_slice(&bytes)?;
            for (item, components) in items.as_object().into_iter().flatten() {
                let Some(pattern) = components["minecraft:provides_pottery_pattern"].as_str()
                else {
                    continue;
                };
                let name = pattern.strip_prefix("minecraft:").unwrap_or(pattern);
                let file = root.join(format!("data/minecraft/decorated_pot_pattern/{name}.json"));
                let Ok(def) = std::fs::read(file) else {
                    continue;
                };
                let def: Value = serde_json::from_slice(&def)?;
                if let Some(asset) = def["asset_id"].as_str() {
                    let asset = asset.strip_prefix("minecraft:").unwrap_or(asset);
                    renderer
                        .pot_patterns
                        .insert(item.clone(), format!("entity/decorated_pot/{asset}"));
                }
            }
        }
        Ok(renderer)
    }
    fn new(models: HashMap<&'static str, ModelPart>, registry: &BlockRegistry) -> Self {
        let kinds = (0..registry.len() as u32)
            .map(|id| {
                registry.state(BlockStateId(id)).map_or(Kind::None, |s| {
                    let prop = |k: &str| s.properties.get(k).cloned();
                    classify(&s.name, &prop)
                })
            })
            .collect();
        Self {
            models,
            kinds,
            books: HashMap::new(),
            random: LegacyRandom::new(0x5EED_B00C),
            pot_patterns: HashMap::new(),
        }
    }
    /// The standing banner's pole-and-bar and flag models (`BannerRenderer.submitSpecial`).
    pub fn banner_models(&self) -> Option<(&ModelPart, &ModelPart)> {
        Some((
            self.models.get("standing_banner")?,
            self.models.get("standing_banner_flag")?,
        ))
    }
    fn kind(&self, state: u32) -> &Kind {
        self.kinds.get(state as usize).unwrap_or(&Kind::None)
    }
    /// Client-side block-entity ticks (enchanting-table books).
    pub fn tick<'a>(
        &mut self,
        blocks: impl Iterator<Item = &'a ([i32; 3], u32)>,
        player: Option<[f64; 3]>,
    ) {
        let mut seen = Vec::new();
        for (pos, state) in blocks {
            if matches!(self.kind(*state), Kind::EnchantingTable) {
                seen.push(*pos);
            }
        }
        self.books.retain(|p, _| seen.contains(p));
        for pos in seen {
            let book = self.books.entry(pos).or_default();
            book.tick(pos, player, &mut self.random);
        }
    }
    pub fn render<'a>(
        &self,
        scene: &Scene,
        blocks: impl Iterator<Item = &'a ([i32; 3], u32)>,
        out: &mut Vec<Vertex>,
    ) {
        let lights = diffuse_lights(scene.nether);
        for (pos, state) in blocks {
            let kind = self.kind(*state);
            if matches!(kind, Kind::None) {
                continue;
            }
            let center = DVec3::new(
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            );
            if center.distance_squared(scene.camera) > VIEW_DISTANCE * VIEW_DISTANCE {
                continue;
            }
            let origin = (DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                - scene.camera)
                .as_vec3();
            let base = Mat4::from_translation(origin);
            let packed = scene.world.light(pos[0], pos[1], pos[2]);
            self.render_one(scene, *pos, kind, base, packed, lights, out);
        }
    }
    /// One block entity at `pos` drawn under `base` with the given packed light
    /// (the parity fixture's entry point).
    pub fn render_block(
        &self,
        scene: &Scene,
        pos: [i32; 3],
        state: u32,
        base: Mat4,
        packed: u8,
        out: &mut Vec<Vertex>,
    ) {
        let kind = self.kind(state);
        let lights = diffuse_lights(scene.nether);
        self.render_one(scene, pos, kind, base, packed, lights, out);
    }
    /// Whether this block state has a block-entity renderer here.
    pub fn renders(&self, state: u32) -> bool {
        !matches!(self.kind(state), Kind::None)
    }
    /// Sets an enchanting table's book fields (`EnchantingTableBlockEntity`):
    /// time, flip, oFlip, open, oOpen, rot, oRot.
    pub fn set_book(&mut self, pos: [i32; 3], time: i32, fields: [f32; 6]) {
        let [flip, o_flip, open, o_open, rot, o_rot] = fields;
        let book = self.books.entry(pos).or_default();
        *book = Book {
            time,
            flip,
            o_flip,
            open,
            o_open,
            rot,
            o_rot,
            ..*book
        };
    }
    /// One block entity of `state` drawn under `base` (minecart contents).
    pub fn render_state(
        &self,
        scene: &Scene,
        state: u32,
        base: Mat4,
        packed: u8,
        out: &mut Vec<Vertex>,
    ) {
        let kind = self.kind(state);
        if !matches!(kind, Kind::None) {
            let lights = diffuse_lights(scene.nether);
            self.render_one(scene, [i32::MAX; 3], kind, base, packed, lights, out);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn render_one(
        &self,
        scene: &Scene,
        pos: [i32; 3],
        kind: &Kind,
        base: Mat4,
        packed: u8,
        lights: (Vec3, Vec3),
        out: &mut Vec<Vertex>,
    ) {
        let sprite = |name: &str| scene.sprites.get(name).copied();
        let paint = |name: &str, tint: u32, packed: u8| {
            sprite(name).map(|s| Paint {
                sprite: s,
                tint,
                light: vertex_light(packed),
                lights,
            })
        };
        let none = |_: &str, _: &mut [f32; 9]| {};
        match kind {
            Kind::None => {}
            Kind::Sign {
                hanging,
                wall,
                angle,
            } => {
                let Some(data) = (scene.sign)(pos) else {
                    return;
                };
                let s = Vec3::new(1.0, -1.0, 1.0);
                for (front, text) in [(true, &data.front), (false, &data.back)] {
                    if text.is_empty() {
                        continue;
                    }
                    let mut m = base;
                    if *hanging {
                        // HangingSignRenderer.textTransformation.
                        m *= Mat4::from_translation(Vec3::new(0.5, 0.9375, 0.5))
                            * Mat4::from_rotation_y((-angle).to_radians())
                            * Mat4::from_translation(Vec3::new(0.0, -0.3125, 0.0));
                        if !front {
                            m *= Mat4::from_rotation_y(PI);
                        }
                        m *= Mat4::from_translation(Vec3::new(0.0, -0.32, 0.073))
                            * Mat4::from_scale(s * 0.014_062_5);
                    } else {
                        // StandingSignRenderer.textTransformation.
                        m *= Mat4::from_translation(Vec3::splat(0.5))
                            * Mat4::from_rotation_y((-angle).to_radians());
                        if *wall {
                            m *= Mat4::from_translation(Vec3::new(0.0, -0.3125, -0.4375));
                        }
                        if !front {
                            m *= Mat4::from_rotation_y(PI);
                        }
                        m *= Mat4::from_translation(Vec3::new(0.0, 1.0 / 3.0, 0.046_666_667))
                            * Mat4::from_scale(s * 0.010_416_667);
                    }
                    let near = DVec3::new(
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]) + 0.5,
                        f64::from(pos[2]) + 0.5,
                    )
                    .distance_squared(scene.camera)
                        < 256.0;
                    let calls = sign_text_calls(scene, data, text, m, packed, near);
                    match scene.texts {
                        Some(sink) => sink.borrow_mut().extend(calls),
                        None => calls.iter().for_each(|c| draw_text(scene, c, out)),
                    }
                }
            }
            Kind::Chest {
                texture,
                part,
                y_rot,
                double_with,
            } => {
                let model = match part {
                    ChestPart::Single => "chest",
                    ChestPart::Left => "double_chest_left",
                    ChestPart::Right => "double_chest_right",
                };
                let (Some(model), Some(_)) = (self.models.get(model), sprite(texture)) else {
                    return;
                };
                // ChestBlock.opennessCombiner / BrightnessCombiner: both halves.
                let mut open = (scene.lid)(pos);
                let mut packed = packed;
                if let Some(d) = double_with {
                    let other = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                    open = open.max((scene.lid)(other));
                    let l = scene.world.light(other[0], other[1], other[2]);
                    packed = (packed >> 4).max(l >> 4) << 4 | (packed & 15).max(l & 15);
                }
                let open = 1.0 - (1.0 - open).powi(3);
                let m = base
                    * around(
                        Mat4::from_rotation_y((-y_rot).to_radians()),
                        Vec3::new(0.5, 0.0, 0.5),
                    );
                let lid = -(open * PI / 2.0);
                let adjust = move |name: &str, p: &mut [f32; 9]| {
                    if name == "lid" || name == "lock" {
                        p[3] = lid;
                    }
                };
                if let Some(paint) = paint(texture, 0xFFFFFF, packed) {
                    draw_part(model, "root", m, &adjust, paint, out);
                }
            }
            Kind::Shulker { texture, facing } => {
                let (Some(model), Some(paint)) = (
                    self.models.get("shulker_box"),
                    paint(texture, 0xFFFFFF, packed),
                ) else {
                    return;
                };
                let m = base
                    * Mat4::from_translation(Vec3::splat(0.5))
                    * Mat4::from_scale(Vec3::splat(0.9995))
                    * Mat4::from_quat(direction_rotation(facing))
                    * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0))
                    * Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0));
                // ShulkerBoxModel.setupAnim(getProgress(partial)): the lid rises and turns.
                let (progress, old) = (scene.anim)(pos).shulker;
                let progress = old + scene.partial * (progress - old);
                let lid = move |name: &str, p: &mut [f32; 9]| {
                    if name == "lid" {
                        p[0] = 0.0;
                        p[1] = 24.0 - progress * 0.5 * 16.0;
                        p[2] = 0.0;
                        p[4] = 270.0 * progress * (PI / 180.0);
                    }
                };
                draw_part(model, "root", m, &lid, paint, out);
            }
            Kind::Banner { color, wall, angle } => {
                let (main, flag) = if *wall {
                    ("wall_banner", "wall_banner_flag")
                } else {
                    ("standing_banner", "standing_banner_flag")
                };
                let (Some(main), Some(flag)) = (self.models.get(main), self.models.get(flag))
                else {
                    return;
                };
                let m = base
                    * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
                    * Mat4::from_rotation_y((-angle).to_radians())
                    * Mat4::from_scale(Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0));
                let time = scene.game_time;
                let phase = ((i64::from(pos[0]) * 7
                    + i64::from(pos[1]) * 9
                    + i64::from(pos[2]) * 13
                    + time)
                    .rem_euclid(100) as f32
                    + scene.partial)
                    / 100.0;
                let wave = move |name: &str, p: &mut [f32; 9]| {
                    if name == "flag" {
                        p[3] = (-0.0125 + 0.01 * (2.0 * PI * phase).mcos()) * PI;
                    }
                };
                if let Some(paint) = paint("entity/banner/banner_base", 0xFFFFFF, packed) {
                    draw_part(main, "root", m, &none, paint, out);
                    draw_part(flag, "root", m, &wave, paint, out);
                }
                // BannerRenderer.submitPatterns: the base pattern layer in the dye colour,
                // then up to 16 pattern layers in order.
                if let Some(paint) = paint("entity/banner/base", *color, packed) {
                    draw_part(flag, "root", m, &wave, paint, out);
                }
                for (texture, tint) in (scene.banner)(pos).iter().take(16) {
                    if let Some(paint) = paint(texture, *tint, packed) {
                        draw_part(flag, "root", m, &wave, paint, out);
                    }
                }
            }
            Kind::Skull {
                kind,
                wall,
                segment,
            } => {
                let (model, texture) = match kind {
                    SkullType::Skeleton => ("skeleton_skull", "entity/skeleton/skeleton"),
                    SkullType::WitherSkeleton => {
                        ("wither_skeleton_skull", "entity/skeleton/wither_skeleton")
                    }
                    SkullType::Zombie => ("zombie_head", "entity/zombie/zombie"),
                    SkullType::Creeper => ("creeper_head", "entity/creeper/creeper"),
                    SkullType::Dragon => ("dragon_skull", "entity/enderdragon/dragon"),
                    SkullType::Piglin => ("piglin_head", "entity/piglin/piglin"),
                    SkullType::Player => ("player_head", "entity/player/slim/steve"),
                };
                let (Some(model), Some(paint)) =
                    (self.models.get(model), paint(texture, 0xFFFFFF, packed))
                else {
                    return;
                };
                let m = match wall {
                    Some(facing) => {
                        let s = step(facing);
                        base * Mat4::from_translation(Vec3::new(
                            0.5 - s[0] as f32 * 0.25,
                            0.25,
                            0.5 - s[2] as f32 * 0.25,
                        )) * Mat4::from_rotation_y((-y_rot(opposite(facing))).to_radians())
                            * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
                    }
                    None => {
                        base * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
                            * Mat4::from_rotation_y((-(f32::from(*segment) * 22.5)).to_radians())
                            * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
                    }
                };
                // SkullBlockEntity.getAnimation: the dragon jaw and piglin ears
                // (DragonHeadModel / PiglinHeadModel use double-precision Math).
                let (animating, count) = (scene.anim)(pos).skull;
                let t = count as f32 + if animating { scene.partial } else { 0.0 };
                let anim = move |name: &str, p: &mut [f32; 9]| match name {
                    "jaw" => p[3] = (f64::from(t * PI * 0.2).sin() + 1.0) as f32 * 0.2,
                    "left_ear" => {
                        p[5] = -((f64::from(t * PI * 0.2 * 1.2).cos() + 2.5) as f32) * 0.2;
                    }
                    "right_ear" => p[5] = (f64::from(t * PI * 0.2).cos() + 2.5) as f32 * 0.2,
                    _ => {}
                };
                draw_part(model, "root", m, &anim, paint, out);
            }
            Kind::Bell => {
                let (Some(model), Some(paint)) = (
                    self.models.get("bell"),
                    paint("entity/bell/bell_body", 0xFFFFFF, packed),
                ) else {
                    return;
                };
                // BellModel.setupAnim(State(ticks + partial, shakeDirection)).
                let a = (scene.anim)(pos);
                let ticks = a.bell_ticks as f32 + scene.partial;
                let (mut x_rot, mut z_rot) = (0.0, 0.0);
                if let Some(dir) = a.bell_shake {
                    let base_rot = (ticks / PI).msin() / (4.0 + ticks / 3.0);
                    match dir {
                        "north" => x_rot = -base_rot,
                        "south" => x_rot = base_rot,
                        "east" => z_rot = -base_rot,
                        "west" => z_rot = base_rot,
                        _ => {}
                    }
                }
                let shake = move |name: &str, p: &mut [f32; 9]| {
                    if name == "bell_body" {
                        p[3] = x_rot;
                        p[5] = z_rot;
                    }
                };
                draw_part(model, "root", base, &shake, paint, out);
            }
            Kind::DecoratedPot { y_rot } => {
                let (Some(base_model), Some(sides)) = (
                    self.models.get("decorated_pot_base"),
                    self.models.get("decorated_pot_sides"),
                ) else {
                    return;
                };
                let a = (scene.anim)(pos);
                let mut m = base
                    * around(
                        Mat4::from_rotation_y((180.0 - y_rot).to_radians()),
                        Vec3::splat(0.5),
                    );
                // DecoratedPotRenderer.submit: the wobble tilts around the base centre.
                let pivot = Vec3::new(0.5, 0.0, 0.5);
                if let Some((positive, progress)) = a.wobble
                    && (0.0..=1.0).contains(&progress)
                {
                    if positive {
                        let delta = progress * (PI * 2.0);
                        let tilt_x = -1.5 * (delta.mcos() + 0.5) * (delta / 2.0).msin();
                        m *= around(Mat4::from_rotation_x(tilt_x * 0.015_625), pivot);
                        let tilt_z = delta.msin();
                        m *= around(Mat4::from_rotation_z(tilt_z * 0.015_625), pivot);
                    } else {
                        let turn = (-progress * 3.0 * PI).msin() * 0.125;
                        let decay = 1.0 - progress;
                        m *= around(Mat4::from_rotation_y(turn * decay), pivot);
                    }
                }
                if let Some(paint) =
                    paint("entity/decorated_pot/decorated_pot_base", 0xFFFFFF, packed)
                {
                    draw_part(base_model, "root", m, &none, paint, out);
                }
                // PotDecorations (back, left, right, front); items without a pattern use the blank side.
                for (side, item) in ["back", "left", "right", "front"].iter().zip(&a.sherds) {
                    let texture = item
                        .as_ref()
                        .and_then(|i| self.pot_patterns.get(i))
                        .map_or("entity/decorated_pot/decorated_pot_side", String::as_str);
                    if let (Some(part), Some(paint)) =
                        (sides.child(side), paint(texture, 0xFFFFFF, packed))
                    {
                        draw_part(part, side, m, &none, paint, out);
                    }
                }
            }
            Kind::Conduit => {
                let (ticks, active, hunting, raw_rotation) = (scene.anim)(pos).conduit;
                // ConduitRenderer.extractRenderState.
                let rotation = (raw_rotation + if active { scene.partial } else { 0.0 }) * -0.0375;
                let tex = |name: &str| paint(name, 0xFFFFFF, packed);
                if !active {
                    let (Some(model), Some(paint)) =
                        (self.models.get("conduit_shell"), tex("entity/conduit/base"))
                    else {
                        return;
                    };
                    let m = base
                        * Mat4::from_translation(Vec3::splat(0.5))
                        * Mat4::from_rotation_y(rotation.to_radians());
                    draw_part(model, "root", m, &none, paint, out);
                    return;
                }
                let anim_time = ticks as f32 + scene.partial;
                let phase = ticks / 66 % 3;
                let degrees = rotation * (180.0 / PI);
                let hh = (anim_time * 0.1).msin() / 2.0 + 0.5;
                let hh = hh * hh + hh;
                let lift = Vec3::new(0.5, 0.3 + hh * 0.2, 0.5);
                if let (Some(model), Some(paint)) =
                    (self.models.get("conduit_cage"), tex("entity/conduit/cage"))
                {
                    let axis = Vec3::new(0.5, 1.0, 0.5).normalize();
                    let m = base
                        * Mat4::from_translation(lift)
                        * Mat4::from_quat(Quat::from_axis_angle(axis, degrees * (PI / 180.0)));
                    draw_part(model, "root", m, &none, paint, out);
                }
                let wind = if phase == 1 {
                    "entity/conduit/wind_vertical"
                } else {
                    "entity/conduit/wind"
                };
                if let (Some(model), Some(paint)) = (self.models.get("conduit_wind"), tex(wind)) {
                    let turn = match phase {
                        1 => Mat4::from_rotation_x(PI / 2.0),
                        2 => Mat4::from_rotation_z(PI / 2.0),
                        _ => Mat4::IDENTITY,
                    };
                    let center = base * Mat4::from_translation(Vec3::splat(0.5));
                    draw_part(model, "root", center * turn, &none, paint, out);
                    let inner = center
                        * Mat4::from_scale(Vec3::splat(0.875))
                        * Mat4::from_quat(Quat::from_euler(glam::EulerRot::XYZ, PI, 0.0, PI));
                    draw_part(model, "root", inner, &none, paint, out);
                }
                let eye = if hunting {
                    "entity/conduit/open_eye"
                } else {
                    "entity/conduit/closed_eye"
                };
                if let (Some(model), Some(paint)) = (self.models.get("conduit_eye"), tex(eye)) {
                    let m = base
                        * Mat4::from_translation(lift)
                        * Mat4::from_scale(Vec3::splat(0.5))
                        * Mat4::from_quat(scene.orientation)
                        * Mat4::from_quat(Quat::from_rotation_z(PI) * Quat::from_rotation_y(PI))
                        * Mat4::from_scale(Vec3::splat(1.333_333_4));
                    draw_part(model, "root", m, &none, paint, out);
                }
            }
            Kind::EnchantingTable => {
                let (Some(model), Some(paint)) = (
                    self.models.get("book"),
                    paint("entity/enchantment/enchanting_table_book", 0xFFFFFF, packed),
                ) else {
                    return;
                };
                let b = self.books.get(&pos).copied().unwrap_or_default();
                let a = scene.partial;
                let flip = b.o_flip + (b.flip - b.o_flip) * a;
                let open = b.o_open + (b.open - b.o_open) * a;
                let time = b.time as f32 + a;
                let y_rot = b.o_rot + wrap(b.rot - b.o_rot) * a;
                let m = base
                    * Mat4::from_translation(Vec3::new(0.5, 0.75, 0.5))
                    * Mat4::from_translation(Vec3::new(0.0, 0.1 + (time * 0.1).msin() * 0.01, 0.0))
                    * Mat4::from_rotation_y(-y_rot)
                    * Mat4::from_rotation_z(80f32.to_radians());
                let ff1 = (frac(flip + 0.25) * 1.6 - 0.3).clamp(0.0, 1.0);
                let ff2 = (frac(flip + 0.75) * 1.6 - 0.3).clamp(0.0, 1.0);
                // BookModel.State.forAnimation.
                let openness = ((time * 0.02).msin() * 0.1 + 1.25) * open;
                draw_part(model, "root", m, &book_pose(openness, ff1, ff2), paint, out);
            }
            Kind::Lectern { y_rot } => {
                let (Some(model), Some(paint)) = (
                    self.models.get("book"),
                    paint("entity/enchantment/enchanting_table_book", 0xFFFFFF, packed),
                ) else {
                    return;
                };
                let m = base
                    * Mat4::from_translation(Vec3::new(0.5, 1.0625, 0.5))
                    * Mat4::from_rotation_y((-y_rot).to_radians())
                    * Mat4::from_rotation_z(67.5f32.to_radians())
                    * Mat4::from_translation(Vec3::new(0.0, -0.125, 0.0));
                // LecternRenderer.BOOK_STATE = forAnimation(0, 0.1, 0.9, 1.2).
                draw_part(
                    model,
                    "root",
                    m,
                    &book_pose(1.25 * 1.2, 0.1, 0.9),
                    paint,
                    out,
                );
            }
            Kind::CopperGolem {
                texture,
                pose,
                facing,
            } => {
                let key = match *pose {
                    "running" => "copper_golem_running",
                    "sitting" => "copper_golem_sitting",
                    "star" => "copper_golem_star",
                    _ => "copper_golem",
                };
                let (Some(model), Some(paint)) =
                    (self.models.get(key), paint(texture, 0xFFFFFF, packed))
                else {
                    return;
                };
                let m = base
                    * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
                    * Mat4::from_rotation_y((-y_rot(opposite(facing))).to_radians());
                // CopperGolemStatueModel.setupAnim: root.y = 0, root.zRot = PI.
                let statue = |name: &str, p: &mut [f32; 9]| {
                    if name == "root" {
                        p[1] = 0.0;
                        p[5] = PI;
                    }
                };
                draw_part(model, "root", m, &statue, paint, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_vanilla_block_names() {
        let props = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(a, _)| *a == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let k = classify(
            "minecraft:light_gray_shulker_box",
            &props(&[("facing", "up")]),
        );
        assert!(
            matches!(k, Kind::Shulker { ref texture, .. } if texture == "entity/shulker/shulker_light_gray")
        );
        let k = classify("minecraft:gray_wall_banner", &props(&[("facing", "west")]));
        assert!(matches!(k, Kind::Banner { color: 0x474F52, wall: true, angle } if angle == 90.0));
        let k = classify(
            "minecraft:wither_skeleton_wall_skull",
            &props(&[("facing", "north")]),
        );
        assert!(matches!(
            k,
            Kind::Skull {
                kind: SkullType::WitherSkeleton,
                wall: Some("north"),
                ..
            }
        ));
        let k = classify("minecraft:piglin_head", &props(&[("rotation", "4")]));
        assert!(matches!(
            k,
            Kind::Skull {
                kind: SkullType::Piglin,
                wall: None,
                segment: 4
            }
        ));
        let k = classify(
            "minecraft:chest",
            &props(&[("facing", "north"), ("type", "left")]),
        );
        // A north-facing left half connects to the east (clockwise).
        assert!(matches!(
            k,
            Kind::Chest {
                part: ChestPart::Left,
                double_with: Some([1, 0, 0]),
                texture: "entity/chest/normal_left",
                ..
            }
        ));
        let k = classify(
            "minecraft:waxed_oxidized_copper_chest",
            &props(&[("facing", "south"), ("type", "single")]),
        );
        assert!(matches!(
            k,
            Kind::Chest {
                texture: "entity/chest/copper_oxidized",
                ..
            }
        ));
        assert!(matches!(
            classify("minecraft:lectern", &props(&[("has_book", "false")])),
            Kind::None
        ));
        assert!(matches!(
            classify("minecraft:furnace", &props(&[])),
            Kind::None
        ));
    }
    #[test]
    fn mix_light_matches_shader() {
        let lights = diffuse_lights(false);
        // Straight up: both lights contribute.
        let up = diffuse(Vec3::Y, lights);
        let expected = ((lights.0.y + lights.1.y) * 0.6 + 0.4).min(1.0);
        assert!((up - expected).abs() < 1e-6);
        // Straight down: ambient only.
        assert!((diffuse(-Vec3::Y, lights) - 0.4).abs() < 1e-6);
    }
    #[test]
    fn part_poses_compose_like_model_part() {
        let part = ModelPart {
            pose: [16.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: vec![[
                [0.0, 0.0, 0.0, 0.0, 0.0],
                [16.0, 0.0, 0.0, 1.0, 0.0],
                [16.0, 0.0, 16.0, 1.0, 1.0],
                [0.0, 0.0, 16.0, 0.0, 1.0],
            ]],
            children: vec![],
        };
        let mut out = Vec::new();
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.5,
            v1: 0.5,
            w: 1,
            h: 1,
        };
        draw_part(
            &part,
            "root",
            Mat4::IDENTITY,
            &|_, _| {},
            Paint {
                sprite: rect,
                tint: 0xFFFFFF,
                light: 0,
                lights: diffuse_lights(false),
            },
            &mut out,
        );
        assert_eq!(out.len(), 4);
        assert_eq!(out[1].pos, [2.0, 0.0, 0.0]);
        assert_eq!(out[2].uv, [0.5, 0.5]);
    }
}

/// `BeaconRenderer.submit`: for each beam section (ARGB colour, height) the
/// rotating solid beam and its translucent glow, the last section 2048
/// blocks tall. `origin` is the beacon block's corner relative to the camera,
/// `time` the animation time (game time mod 40 plus partial tick) and `scale`
/// the radius scale. The sprite's v range rides in the light bytes (the
/// entity shader wraps v for layer 6).
pub fn beacon_beam(
    origin: Vec3,
    sections: &[(u32, i32)],
    time: f32,
    scale: f32,
    s: Rect,
    out: &mut Vec<Vertex>,
) {
    let q = |v: f32| (v * 65535.0).round().clamp(0.0, 65535.0) as u32;
    let wrap = q(s.v0) | q(s.v1 - s.v0) << 16;
    let o = origin + Vec3::new(0.5, 0.0, 0.5);
    let mut start = 0i32;
    for (i, &(color, height)) in sections.iter().enumerate() {
        let height = if i == sections.len() - 1 {
            2048
        } else {
            height
        };
        let end = start + height;
        let scroll = if height < 0 { time } else { -time };
        let v_off = frac(scroll * 0.2 - (scroll * 0.1).floor());
        let abgr =
            |a: u32| a << 24 | (color & 255) << 16 | (color >> 8 & 255) << 8 | (color >> 16 & 255);
        // Solid beam: radius 0.2 * scale, rotated with time.
        let r = 0.2 * scale;
        let rot = glam::Mat3::from_rotation_y((time * 2.25 - 45.0).to_radians());
        let corners = [
            rot * Vec3::new(0.0, 0.0, r),
            rot * Vec3::new(r, 0.0, 0.0),
            rot * Vec3::new(-r, 0.0, 0.0),
            rot * Vec3::new(0.0, 0.0, -r),
        ];
        let v2 = -1.0 + v_off;
        let v1 = height as f32 * (0.5 / r) + v2;
        beam_part(out, o, start, end, corners, [v1, v2], abgr(255), s, wrap);
        // Glow: radius 0.25 * scale, translucent.
        let g = 0.25 * scale;
        let corners = [
            Vec3::new(-g, 0.0, -g),
            Vec3::new(g, 0.0, -g),
            Vec3::new(-g, 0.0, g),
            Vec3::new(g, 0.0, g),
        ];
        let v1 = height as f32 + v2;
        beam_part(out, o, start, end, corners, [v1, v2], abgr(32), s, wrap);
        start = end;
    }
}
/// `BeaconRenderer.renderPart`: four quads between the corners (wn, en, ws, es).
#[allow(clippy::too_many_arguments)]
fn beam_part(
    out: &mut Vec<Vertex>,
    o: Vec3,
    start: i32,
    end: i32,
    [wn, en, ws, es]: [Vec3; 4],
    [v1, v2]: [f32; 2],
    color: u32,
    s: Rect,
    wrap: u32,
) {
    for (a, b) in [(wn, en), (es, ws), (en, es), (ws, wn)] {
        let at = |c: Vec3, y: i32| o + c + Vec3::new(0.0, y as f32, 0.0);
        for (p, uv) in [
            (at(a, end), [s.u1, v1]),
            (at(a, start), [s.u1, v2]),
            (at(b, start), [s.u0, v2]),
            (at(b, end), [s.u0, v1]),
        ] {
            out.push(Vertex {
                pos: p.to_array(),
                uv,
                layer: 6,
                color,
                light: wrap,
            });
        }
    }
}
