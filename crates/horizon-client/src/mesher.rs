//! Section meshing on worker threads. Faces follow the baked blockstate models
//! with vanilla's neighbour culling; vertex lighting follows
//! `ModelBlockRenderer`'s smooth-lighting blend (four samples per corner, zero
//! samples replaced by the centre) and ambient-occlusion shade brightness.
//! Fluids follow `FluidRenderer` (corner heights, still/flowing sprites).
use crate::{
    assets::Assets,
    biome::Biomes,
    blocks::{BlockFacts, Fluid, RenderShape},
    chunk::{Chunk, MIN_Y},
    models::catalog::BakedFace,
};
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::{block_render::FaceVisibility, registry::BlockStateId};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Section-relative position.
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
    /// RGBA8: tint multiplied by directional shade and ambient occlusion.
    pub color: u32,
    /// Smooth light, 16x fixed point: sky in bits 0..8, block in bits 8..16.
    pub light: u32,
}
#[derive(Default)]
pub struct SectionMesh {
    pub solid: Vec<Vertex>,
    pub translucent: Vec<Vertex>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tint {
    None,
    Grass,
    TallGrassUpper,
    Foliage,
    DryFoliage,
    Water,
    Constant(u32),
    Redstone(u8),
    Stem(u8),
}
/// Per-state data the mesher needs on every block, flattened for speed.
#[derive(Clone, Copy)]
struct Info {
    solid: bool,
    /// `isCollisionShapeFullBlock` drives the 0.2 AO shade brightness.
    full: bool,
    /// Side blocks that are transparent let the AO corner sample through.
    translucent: bool,
    emission: u8,
    fluid: Fluid,
    level: u8,
    model: bool,
    tints: [Tint; 2],
}
pub struct MeshTables {
    info: Vec<Info>,
    pub visibility: FaceVisibility,
    water_still: u32,
    water_flow: u32,
    lava_still: u32,
    lava_flow: u32,
}
fn redstone_color(power: u8) -> u32 {
    // RedstoneWireBlock.COLORS
    let f = f32::from(power) / 15.0;
    let r = f * 0.6 + if f > 0.0 { 0.4 } else { 0.3 };
    let g = (f * f * 0.7 - 0.5).clamp(0.0, 1.0);
    let b = (f * f * 0.6 - 0.7).clamp(0.0, 1.0);
    ((r * 255.0) as u32) << 16 | ((g * 255.0) as u32) << 8 | (b * 255.0) as u32
}
impl MeshTables {
    pub fn new(
        assets: &Assets,
        facts: &BlockFacts,
        visibility: FaceVisibility,
    ) -> crate::Result<Self> {
        let layer = |name: &str| -> crate::Result<u32> {
            assets
                .sprite_layers
                .get(name)
                .copied()
                .ok_or_else(|| format!("missing fluid sprite {name}").into())
        };
        let info = facts
            .states
            .iter()
            .enumerate()
            .map(|(id, s)| {
                let props = assets
                    .registry
                    .state(BlockStateId(id as u32))
                    .map(|s| &s.properties);
                let prop = |k: &str| -> u8 {
                    props
                        .and_then(|p| p.get(k))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0)
                };
                let name = s.name.strip_prefix("minecraft:").unwrap_or(&s.name);
                let tints = match name {
                    "large_fern" | "tall_grass" => {
                        if props.and_then(|p| p.get("half")).map(String::as_str) == Some("upper") {
                            [Tint::TallGrassUpper; 2]
                        } else {
                            [Tint::Grass; 2]
                        }
                    }
                    "fern" | "short_grass" | "potted_fern" | "bush" | "grass_block"
                    | "sugar_cane" => [Tint::Grass; 2],
                    "pink_petals" | "wildflowers" => [Tint::None, Tint::Grass],
                    "spruce_leaves" => [Tint::Constant(0x61_9961); 2],
                    "birch_leaves" => [Tint::Constant(0x80_A755); 2],
                    "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
                    | "vine" | "mangrove_leaves" => [Tint::Foliage; 2],
                    "leaf_litter" => [Tint::DryFoliage; 2],
                    "water_cauldron" | "water" | "bubble_column" => [Tint::Water; 2],
                    "redstone_wire" => [Tint::Redstone(prop("power")); 2],
                    "attached_melon_stem" | "attached_pumpkin_stem" => {
                        [Tint::Constant(0xE0_C71C); 2]
                    }
                    "melon_stem" | "pumpkin_stem" => [Tint::Stem(prop("age")); 2],
                    "lily_pad" => [Tint::Constant(0x20_8030); 2],
                    _ => [Tint::None; 2],
                };
                Info {
                    solid: s.solid_render,
                    full: s.full_collision,
                    translucent: !s.solid_render || s.dampening == 0,
                    emission: s.emission,
                    fluid: s.fluid,
                    level: s.fluid_level,
                    model: s.render_shape == RenderShape::Model,
                    tints,
                }
            })
            .collect();
        Ok(Self {
            info,
            visibility,
            water_still: layer("minecraft:block/water_still")?,
            water_flow: layer("minecraft:block/water_flow")?,
            lava_still: layer("minecraft:block/lava_still")?,
            lava_flow: layer("minecraft:block/lava_flow")?,
        })
    }
}
const P: usize = 18;
#[inline]
fn pi(x: i32, y: i32, z: i32) -> usize {
    ((y + 1) as usize * P + (z + 1) as usize) * P + (x + 1) as usize
}
/// Blocks and light of one section plus a one-block border.
struct Padded {
    blocks: Vec<u16>,
    light: Vec<u8>,
}
impl Padded {
    fn new(region: &[[Option<&Chunk>; 3]; 3], sy: i32) -> Self {
        let mut blocks = vec![0u16; P * P * P];
        let mut light = vec![0u8; P * P * P];
        let base_y = MIN_Y + sy * 16;
        for y in -1..17 {
            let wy = base_y + y;
            for z in -1..17i32 {
                for x in -1..17i32 {
                    let chunk = region[((z + 16) >> 4) as usize][((x + 16) >> 4) as usize];
                    let i = pi(x, y, z);
                    if let Some(c) = chunk {
                        blocks[i] = c.get(x & 15, wy, z & 15);
                        light[i] = c.light(x & 15, wy, z & 15);
                    } else {
                        light[i] = 0xF0;
                    }
                }
            }
        }
        Self { blocks, light }
    }
}
#[derive(Clone, Copy)]
struct Colors {
    grass: [u32; 256],
    tall_upper: [u32; 256],
    foliage: [u32; 256],
    dry: [u32; 256],
    water: [u32; 256],
}
fn blend(region: &[[Option<&Chunk>; 3]; 3], biomes: &Biomes, cx: i32, cz: i32, y: i32) -> Colors {
    // Vanilla blends over a (2r+1)^2 area; r = 2 is the default option.
    const R: i32 = 2;
    let biome_at = |x: i32, z: i32| -> u16 {
        region[((z + 16) >> 4) as usize][((x + 16) >> 4) as usize]
            .map_or(0, |c| c.biome(x & 15, y, z & 15))
    };
    let mut colors = Colors {
        grass: [0; 256],
        tall_upper: [0; 256],
        foliage: [0; 256],
        dry: [0; 256],
        water: [0; 256],
    };
    let avg = |f: &dyn Fn(u16, i32, i32) -> u32, x: i32, z: i32| -> u32 {
        let (mut r, mut g, mut b) = (0, 0, 0);
        for dz in -R..=R {
            for dx in -R..=R {
                let c = f(biome_at(x + dx, z + dz), x + dx, z + dz);
                r += c >> 16 & 255;
                g += c >> 8 & 255;
                b += c & 255;
            }
        }
        let n = ((2 * R + 1) * (2 * R + 1)) as u32;
        (r / n) << 16 | (g / n) << 8 | b / n
    };
    for z in 0..16 {
        for x in 0..16 {
            let i = (z * 16 + x) as usize;
            let (wx, wz) = (cx * 16 + x, cz * 16 + z);
            colors.grass[i] = avg(
                &|b, dx, dz| biomes.grass(b, f64::from(wx + dx - x), f64::from(wz + dz - z)),
                x,
                z,
            );
            colors.tall_upper[i] = colors.grass[i];
            colors.foliage[i] = avg(&|b, _, _| biomes.get(b).foliage, x, z);
            colors.dry[i] = avg(&|b, _, _| biomes.get(b).dry_foliage, x, z);
            colors.water[i] = avg(&|b, _, _| biomes.get(b).water, x, z);
        }
    }
    colors
}
fn shade(direction: horizon_assets::model::Direction) -> f32 {
    use horizon_assets::model::Direction as D;
    match direction {
        D::Down => 0.5,
        D::Up => 1.0,
        D::North | D::South => 0.8,
        D::West | D::East => 0.6,
    }
}
fn pack_color(rgb: u32, brightness: f32) -> u32 {
    let r = ((rgb >> 16 & 255) as f32 * brightness) as u32;
    let g = ((rgb >> 8 & 255) as f32 * brightness) as u32;
    let b = ((rgb & 255) as f32 * brightness) as u32;
    0xFF00_0000 | b << 16 | g << 8 | r
}
fn pack_light(sky: f32, block: f32) -> u32 {
    (sky * 16.0).clamp(0.0, 255.0) as u32 | ((block * 16.0).clamp(0.0, 255.0) as u32) << 8
}
pub struct Mesher<'a> {
    pub assets: &'a Assets,
    pub tables: &'a MeshTables,
    pub biomes: &'a Biomes,
    pub visibility: FaceVisibility,
}
impl Mesher<'_> {
    /// `region[dz][dx]` are the chunks around the meshed column (centre 1,1).
    pub fn mesh(&mut self, region: &[[Option<&Chunk>; 3]; 3], sy: i32) -> SectionMesh {
        let mut out = SectionMesh::default();
        let center = region[1][1].expect("centre chunk");
        let section = &center.sections[sy as usize];
        if section.is_uniform(0) {
            return out;
        }
        let pad = Padded::new(region, sy);
        let base_y = MIN_Y + sy * 16;
        let colors = blend(region, self.biomes, center.x, center.z, base_y + 8);
        let info = &self.tables.info;
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let id = pad.blocks[pi(x, y, z)];
                    if id == 0 {
                        continue;
                    }
                    let state = info[id as usize];
                    if state.solid
                        && Direction::ALL.iter().all(|d| {
                            let [dx, dy, dz] = d.normal();
                            info[pad.blocks[pi(x + dx, y + dy, z + dz)] as usize].solid
                        })
                    {
                        continue;
                    }
                    if state.fluid != Fluid::None {
                        self.fluid(&pad, &colors, &mut out, x, y, z, id);
                    }
                    if !state.model {
                        continue;
                    }
                    let world = BlockPos::new(center.x * 16 + x, base_y + y, center.z * 16 + z);
                    let Ok(models) = self.assets.model_catalog.collect(BlockStateId(id.into()), world)
                    else {
                        continue;
                    };
                    let mut visible = [false; 6];
                    for d in Direction::ALL {
                        let [dx, dy, dz] = d.normal();
                        let neighbor = pad.blocks[pi(x + dx, y + dy, z + dz)];
                        visible[d as usize] = self
                            .visibility
                            .should_render_face(
                                BlockStateId(id.into()),
                                BlockStateId(neighbor.into()),
                                d,
                                true,
                            )
                            .unwrap_or(true);
                    }
                    for (_, model) in models {
                        for face in &model.faces {
                            if face.quad.cull.is_some_and(|d| !visible[d as usize]) {
                                continue;
                            }
                            self.face(
                                &pad,
                                &colors,
                                &mut out,
                                [x, y, z],
                                face,
                                model.ambient_occlusion,
                                state,
                            );
                        }
                    }
                }
            }
        }
        out
    }
    fn tint(state: Info, index: i32, colors: &Colors, x: i32, z: i32) -> u32 {
        if index < 0 {
            return 0xFF_FFFF;
        }
        let column = (z * 16 + x) as usize;
        match state.tints[(index as usize).min(1)] {
            Tint::None => 0xFF_FFFF,
            Tint::Grass => colors.grass[column],
            Tint::TallGrassUpper => colors.tall_upper[column],
            Tint::Foliage => colors.foliage[column],
            Tint::DryFoliage => colors.dry[column],
            Tint::Water => colors.water[column],
            Tint::Constant(c) => c,
            Tint::Redstone(p) => redstone_color(p),
            Tint::Stem(age) => {
                let a = u32::from(age);
                (a * 32) << 16 | (255 - a * 8) << 8 | a * 4
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn face(
        &self,
        pad: &Padded,
        colors: &Colors,
        out: &mut SectionMesh,
        [x, y, z]: [i32; 3],
        face: &BakedFace,
        ao: bool,
        state: Info,
    ) {
        let q = &face.quad;
        let facing = q.facing;
        let normal = Direction::ALL[facing as usize].normal();
        let axis = normal.iter().position(|&n| n != 0).unwrap_or(1);
        let positive = normal[axis] > 0;
        let on_boundary = q.positions.iter().all(|p| {
            if positive {
                p[axis] >= 1.0 - 1e-4
            } else {
                p[axis] <= 1e-4
            }
        });
        let base = if on_boundary {
            [x + normal[0], y + normal[1], z + normal[2]]
        } else {
            [x, y, z]
        };
        let tint = Self::tint(state, q.tint_index, colors, x, z);
        let directional = match q.shade_direction_override {
            Some(d) => shade(d),
            None => shade(facing),
        };
        let info = &self.tables.info;
        let light_at = |p: [i32; 3]| -> u32 {
            let i = pi(p[0], p[1], p[2]);
            let packed = pad.light[i];
            let s = info[pad.blocks[i] as usize];
            let block = (packed & 15).max(s.emission);
            u32::from(packed >> 4) << 16 | u32::from(block)
        };
        let shade_at = |p: [i32; 3]| -> f32 {
            if info[pad.blocks[pi(p[0], p[1], p[2])] as usize].full {
                0.2
            } else {
                1.0
            }
        };
        let open_at = |p: [i32; 3]| -> bool { info[pad.blocks[pi(p[0], p[1], p[2])] as usize].translucent };
        // Tangent axes of the face plane.
        let (ua, va) = match axis {
            0 => (2, 1),
            1 => (0, 2),
            _ => (0, 1),
        };
        let center_light = light_at(base);
        let center_shade = shade_at(base);
        // Corner values for (su, sv) in {0,1}^2 -> (sky, block, ao).
        let mut corners = [[0f32; 3]; 4];
        for (ci, corner) in corners.iter_mut().enumerate() {
            let su = if ci & 1 == 0 { -1 } else { 1 };
            let sv = if ci & 2 == 0 { -1 } else { 1 };
            let mut p1 = base;
            p1[ua] += su;
            let mut p2 = base;
            p2[va] += sv;
            let mut pc = base;
            pc[ua] += su;
            pc[va] += sv;
            let (l1, l2) = (light_at(p1), light_at(p2));
            let (lc, ac) = if open_at(p1) || open_at(p2) {
                (light_at(pc), shade_at(pc))
            } else {
                (l1, shade_at(p1))
            };
            // ModelBlockRenderer.blend: zero samples take the centre value.
            let fix = |v: u32| if v == 0 { center_light } else { v };
            let sum = fix(l1) + fix(l2) + fix(lc) + center_light;
            corner[0] = (sum >> 16) as f32 / 4.0;
            corner[1] = (sum & 0xFFFF) as f32 / 4.0;
            corner[2] = (shade_at(p1) + shade_at(p2) + ac + center_shade) / 4.0;
        }
        let flat_sky = (center_light >> 16) as f32;
        let flat_block = (center_light & 0xFFFF) as f32;
        let target = if face.translucent {
            &mut out.translucent
        } else {
            &mut out.solid
        };
        for (p, uv) in q.positions.iter().zip(q.uv) {
            let (sky, block, occlusion) = if ao {
                let u = p[ua].clamp(0.0, 1.0);
                let v = p[va].clamp(0.0, 1.0);
                let lerp = |k: usize| {
                    let a = corners[0][k] * (1.0 - u) + corners[1][k] * u;
                    let b = corners[2][k] * (1.0 - u) + corners[3][k] * u;
                    a * (1.0 - v) + b * v
                };
                (lerp(0), lerp(1), lerp(2))
            } else {
                (flat_sky, flat_block, 1.0)
            };
            target.push(Vertex {
                pos: [p[0] + x as f32, p[1] + y as f32, p[2] + z as f32],
                uv,
                layer: face.layer,
                color: pack_color(tint, directional * occlusion),
                light: pack_light(sky, block),
            });
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn fluid(&self, pad: &Padded, colors: &Colors, out: &mut SectionMesh, x: i32, y: i32, z: i32, id: u16) {
        let info = &self.tables.info;
        let me = info[id as usize];
        let water = me.fluid == Fluid::Water;
        let same = |p: [i32; 3]| info[pad.blocks[pi(p[0], p[1], p[2])] as usize].fluid == me.fluid;
        let solid = |p: [i32; 3]| {
            let s = info[pad.blocks[pi(p[0], p[1], p[2])] as usize];
            s.solid || s.full
        };
        let height = |p: [i32; 3]| -> f32 {
            let s = info[pad.blocks[pi(p[0], p[1], p[2])] as usize];
            if s.fluid == me.fluid {
                if same([p[0], p[1] + 1, p[2]]) {
                    1.0
                } else {
                    f32::from(s.level) / 9.0
                }
            } else if !(s.solid || s.full) {
                0.0
            } else {
                -1.0
            }
        };
        let own = height([x, y, z]);
        let corner = |a: f32, b: f32, c: [i32; 3]| -> f32 {
            if a >= 1.0 || b >= 1.0 {
                return 1.0;
            }
            let mut sum = [0.0f32; 2];
            let mut add = |h: f32| {
                if h >= 0.8 {
                    sum[0] += h * 10.0;
                    sum[1] += 10.0;
                } else if h >= 0.0 {
                    sum[0] += h;
                    sum[1] += 1.0;
                }
            };
            if a > 0.0 || b > 0.0 {
                let hc = height(c);
                if hc >= 1.0 {
                    return 1.0;
                }
                add(hc);
            }
            add(own);
            add(a);
            add(b);
            sum[0] / sum[1]
        };
        let (nw, sw, se, ne) = if own >= 1.0 {
            (1.0, 1.0, 1.0, 1.0)
        } else {
            let n = height([x, y, z - 1]);
            let s = height([x, y, z + 1]);
            let e = height([x + 1, y, z]);
            let w = height([x - 1, y, z]);
            (
                corner(n, w, [x - 1, y, z - 1]),
                corner(s, w, [x - 1, y, z + 1]),
                corner(s, e, [x + 1, y, z + 1]),
                corner(n, e, [x + 1, y, z - 1]),
            )
        };
        let tint = if water {
            colors.water[(z * 16 + x) as usize]
        } else {
            0xFF_FFFF
        };
        let (still, flow) = if water {
            (self.tables.water_still, self.tables.water_flow)
        } else {
            (self.tables.lava_still, self.tables.lava_flow)
        };
        let light = |p: [i32; 3]| {
            let packed = pad.light[pi(p[0], p[1], p[2])];
            let emission = if water { 0 } else { 15 };
            pack_light(f32::from(packed >> 4), f32::from((packed & 15).max(emission)))
        };
        let target = if water {
            &mut out.translucent
        } else {
            &mut out.solid
        };
        let (fx, fy, fz) = (x as f32, y as f32, z as f32);
        let quad = |target: &mut Vec<Vertex>, v: [([f32; 3], [f32; 2]); 4], layer: u32, b: f32, l: u32, back: bool| {
            for (p, uv) in v {
                target.push(Vertex {
                    pos: p,
                    uv,
                    layer,
                    color: pack_color(tint, b),
                    light: l,
                });
            }
            if back {
                for i in [0, 3, 2, 1] {
                    let (p, uv) = v[i];
                    target.push(Vertex {
                        pos: p,
                        uv,
                        layer,
                        color: pack_color(tint, b),
                        light: l,
                    });
                }
            }
        };
        let above_same = same([x, y + 1, z]);
        if !above_same && !solid([x, y + 1, z]) || !above_same && own < 1.0 {
            let o = 0.001;
            let (h0, h1, h2, h3) = (nw - o, sw - o, se - o, ne - o);
            // Flow direction from neighbour height differences.
            let mut fxv = 0.0f32;
            let mut fzv = 0.0f32;
            if own < 1.0 && me.level < 8 || own < 0.88 {
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let h = height([x + dx, y, z + dz]);
                    if h >= 0.0 {
                        let d = own - h.max(0.0);
                        fxv += dx as f32 * d;
                        fzv += dz as f32 * d;
                    }
                }
            }
            let (layer, uvs) = if fxv.abs() < 1e-4 && fzv.abs() < 1e-4 {
                (still, [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]])
            } else {
                let angle = fzv.atan2(fxv) - std::f32::consts::FRAC_PI_2;
                let (s, c) = (angle.sin() * 0.25, angle.cos() * 0.25);
                // Flowing sprites are sampled over their top-left quarter-scaled region (getU(0.5+..) of 16 units).
                (
                    flow,
                    [
                        [0.5 + (-c - s), 0.5 + (-c + s)],
                        [0.5 + (-c + s), 0.5 + (c + s)],
                        [0.5 + (c + s), 0.5 + (c - s)],
                        [0.5 + (c - s), 0.5 + (-c - s)],
                    ],
                )
            };
            quad(
                target,
                [
                    ([fx, fy + h0, fz], uvs[0]),
                    ([fx, fy + h1, fz + 1.0], uvs[1]),
                    ([fx + 1.0, fy + h2, fz + 1.0], uvs[2]),
                    ([fx + 1.0, fy + h3, fz], uvs[3]),
                ],
                layer,
                1.0,
                light([x, y, z]),
                water && !solid([x, y + 1, z]),
            );
        }
        if !same([x, y - 1, z]) && !solid([x, y - 1, z]) {
            quad(
                target,
                [
                    ([fx, fy + 0.001, fz], [0.0, 0.0]),
                    ([fx + 1.0, fy + 0.001, fz], [1.0, 0.0]),
                    ([fx + 1.0, fy + 0.001, fz + 1.0], [1.0, 1.0]),
                    ([fx, fy + 0.001, fz + 1.0], [0.0, 1.0]),
                ],
                still,
                0.5,
                light([x, y - 1, z]),
                false,
            );
        }
        let o = 0.001;
        for (dx, dz, h0, h1, p0, p1, sh) in [
            (0, -1, nw, ne, [fx, fz + o], [fx + 1.0, fz + o], 0.8),
            (0, 1, se, sw, [fx + 1.0, fz + 1.0 - o], [fx, fz + 1.0 - o], 0.8),
            (-1, 0, sw, nw, [fx + o, fz + 1.0], [fx + o, fz], 0.6),
            (1, 0, ne, se, [fx + 1.0 - o, fz], [fx + 1.0 - o, fz + 1.0], 0.6),
        ] {
            let n = [x + dx, y, z + dz];
            if same(n) || solid(n) {
                continue;
            }
            let v0 = (1.0 - h0) * 0.5;
            let v1 = (1.0 - h1) * 0.5;
            quad(
                target,
                [
                    ([p0[0], fy + h0, p0[1]], [0.0, v0]),
                    ([p1[0], fy + h1, p1[1]], [0.5, v1]),
                    ([p1[0], fy, p1[1]], [0.5, 0.5]),
                    ([p0[0], fy, p0[1]], [0.0, 0.5]),
                ],
                flow,
                sh,
                light([x, y, z]),
                water,
            );
        }
    }
}
/// Snapshot of the 3x3 chunk neighbourhood used by a worker job.
pub type Region = [[Option<Arc<Chunk>>; 3]; 3];
pub fn borrow(region: &Region) -> [[Option<&Chunk>; 3]; 3] {
    std::array::from_fn(|z| std::array::from_fn(|x| region[z][x].as_deref()))
}
