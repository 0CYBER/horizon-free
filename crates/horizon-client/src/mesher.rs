//! Section meshing on worker threads. Faces follow the baked blockstate models
//! with vanilla's neighbour culling; vertex light and colour follow
//! `ModelBlockRenderer`/`BlockModelLighter` (smooth light, ambient occlusion,
//! cardinal shading, tint sources, block offsets; see `lighting`). Fluids
//! follow `FluidRenderer` (see `fluid`).
mod fluid;
mod lighting;
use crate::{
    assets::Assets,
    biome::Biomes,
    blocks::{BlockFacts, RenderShape},
    chunk::{Chunk, MIN_Y},
    models::catalog::BakedFace,
};
use horizon_core::{geometry::Direction, position::BlockPos, random::position_seed};
use horizon_world::{
    block_render::FaceVisibility,
    fluid::{FluidKind, FluidState},
    registry::BlockStateId,
};
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
pub struct SectionMesh {
    pub solid: Vec<Vertex>,
    pub translucent: Vec<Vertex>,
    /// VisGraph: bit `a * 6 + b` is set when faces `a` and `b` (Direction
    /// order) are connected through non-opaque blocks.
    pub visibility: u64,
}
impl Default for SectionMesh {
    fn default() -> Self {
        Self { solid: Vec::new(), translucent: Vec::new(), visibility: ALL_VISIBLE }
    }
}
pub const ALL_VISIBLE: u64 = (1 << 36) - 1;
/// VisGraph.resolve: flood fill non-opaque cells and connect the faces each region touches.
fn visibility(pad: &Padded, info: &[Info]) -> u64 {
    let mut seen = vec![false; 4096];
    let mut result = 0u64;
    let opaque = |x: i32, y: i32, z: i32| info[pad.blocks[pi(x, y, z)] as usize].solid;
    let mut stack = Vec::new();
    for start in 0..4096usize {
        let (x, y, z) = ((start & 15) as i32, (start >> 8) as i32, ((start >> 4) & 15) as i32);
        if seen[start] || opaque(x, y, z) {
            continue;
        }
        let mut faces = 0u8;
        seen[start] = true;
        stack.push(start);
        while let Some(c) = stack.pop() {
            let (x, y, z) = ((c & 15) as i32, (c >> 8) as i32, ((c >> 4) & 15) as i32);
            if y == 0 { faces |= 1 << 0; }
            if y == 15 { faces |= 1 << 1; }
            if z == 0 { faces |= 1 << 2; }
            if z == 15 { faces |= 1 << 3; }
            if x == 0 { faces |= 1 << 4; }
            if x == 15 { faces |= 1 << 5; }
            for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                if !(0..16).contains(&nx) || !(0..16).contains(&ny) || !(0..16).contains(&nz) {
                    continue;
                }
                let n = (ny << 8 | nz << 4 | nx) as usize;
                if !seen[n] && !opaque(nx, ny, nz) {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        for a in 0..6 {
            if faces & (1 << a) != 0 {
                for b in 0..6 {
                    if faces & (1 << b) != 0 {
                        result |= 1 << (a * 6 + b);
                    }
                }
            }
        }
    }
    result
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
    /// `isSolidRender`.
    solid: bool,
    /// `isCollisionShapeFullBlock`: AO faces flush with a full block sample outside it.
    full: bool,
    /// `isLightPermeable`: diagonal AO samples are taken through such side blocks.
    permeable: bool,
    /// `getShadeBrightness` (ambient occlusion contribution).
    shade: f32,
    emission: u8,
    /// `emissiveRendering`: drawn and sampled at full brightness.
    emissive: bool,
    fluid: FluidState,
    model: bool,
    tints: [Tint; 2],
    tint_count: u8,
    /// `OffsetType` as (max horizontal, max vertical): vertical 0 for XZ.
    offset: Option<(f32, f32)>,
    /// `isSolid()` (legacy), for fluid corner heights.
    legacy_solid: bool,
    /// `HalfTransparentBlock`/`LeavesBlock`: water sides show the overlay sprite.
    overlay: bool,
    blocks_flow: bool,
    ice: bool,
    sturdy: u8,
    /// Occlusion face per direction: 0 empty, 1 the full-block singleton, 2 partial.
    face: [u8; 6],
    /// Height from the bottom over which face `d`, flush with the `d` side, covers
    /// the full width (vertical faces: 1 when the whole square is covered).
    cover: [f32; 6],
}
/// Derived per-state facts, exposed for the section-mesh parity gate.
#[derive(Clone, Copy, Debug)]
pub struct MeshFacts {
    pub shade: f32,
    pub emissive: bool,
    pub legacy_solid: bool,
    pub light_permeable: bool,
    pub solid_render: bool,
    pub full_collision: bool,
    pub emission: u8,
    pub overlay: bool,
    pub tint_sources: u8,
}
pub struct MeshTables {
    info: Vec<Info>,
    pub visibility: FaceVisibility,
    water_still: u32,
    water_flow: u32,
    water_overlay: u32,
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
        let mut covers = std::collections::HashMap::new();
        let mut info = Vec::with_capacity(facts.states.len());
        for (id, s) in facts.states.iter().enumerate() {
            let state = assets
                .registry
                .state(BlockStateId(id as u32))
                .ok_or("missing registry state")?;
            let props = &state.properties;
            let prop = |k: &str| -> u8 { props.get(k).and_then(|v| v.parse().ok()).unwrap_or(0) };
            let name = s.name.strip_prefix("minecraft:").unwrap_or(&s.name);
            // BlockColors.createDefault: tint sources by block.
            let (tints, tint_count) = match name {
                "large_fern" | "tall_grass" => {
                    if props.get("half").map(String::as_str) == Some("upper") {
                        ([Tint::TallGrassUpper; 2], 1)
                    } else {
                        ([Tint::Grass; 2], 1)
                    }
                }
                "fern" | "short_grass" | "potted_fern" | "bush" | "grass_block" | "sugar_cane" => {
                    ([Tint::Grass; 2], 1)
                }
                "pink_petals" | "wildflowers" => ([Tint::None, Tint::Grass], 2),
                "spruce_leaves" => ([Tint::Constant(0x61_9961); 2], 1),
                "birch_leaves" => ([Tint::Constant(0x80_A755); 2], 1),
                "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves" | "vine"
                | "mangrove_leaves" => ([Tint::Foliage; 2], 1),
                "leaf_litter" => ([Tint::DryFoliage; 2], 1),
                "water_cauldron" => ([Tint::Water; 2], 1),
                // Their in-world colour is the particle-only source's -1.
                "water" | "bubble_column" => ([Tint::None; 2], 1),
                "redstone_wire" => ([Tint::Redstone(prop("power")); 2], 1),
                "attached_melon_stem" | "attached_pumpkin_stem" => {
                    ([Tint::Constant(0xE0_C71C); 2], 1)
                }
                "melon_stem" | "pumpkin_stem" => ([Tint::Stem(prop("age")); 2], 1),
                "lily_pad" => ([Tint::Constant(0x20_8030); 2], 1),
                _ => ([Tint::None; 2], 0),
            };
            // getShadeBrightness overrides; the default is 0.2 for full collision shapes.
            let overrides = [
                "TransparentBlock",
                "BarrierBlock",
                "LightBlock",
                "StructureVoidBlock",
            ];
            let shade = if overrides.iter().any(|c| s.has_class(c)) {
                1.0
            } else if s.has_class("SoulSandBlock") || s.has_class("MudBlock") {
                0.2
            } else if s.has_class("SnowLayerBlock") {
                if prop("layers") == 8 { 0.2 } else { 1.0 }
            } else if s.full_collision {
                0.2
            } else {
                1.0
            };
            let emissive = name == "magma_block"
                || (matches!(name, "sculk_sensor" | "calibrated_sculk_sensor")
                    && props.get("sculk_sensor_phase").map(String::as_str) == Some("active"));
            // OffsetType.XZ/XYZ with getMaxHorizontalOffset/getMaxVerticalOffset.
            let offset = s.random_offset.then(|| {
                let horizontal = if s.has_class("SpeleothemBlock") {
                    0.125
                } else {
                    0.25
                };
                let vertical = match name {
                    "small_dripleaf" => 0.1,
                    "short_grass" | "fern" | "short_dry_grass" | "tall_dry_grass" => 0.2,
                    _ => 0.0,
                };
                (horizontal, vertical)
            });
            let mut face = [0u8; 6];
            let mut cover = [0f32; 6];
            for d in Direction::ALL {
                let (face_id, shape) = visibility
                    .occlusion_face(BlockStateId(id as u32), d)
                    .ok_or("missing occlusion face")?;
                face[d as usize] = face_id.min(2) as u8;
                cover[d as usize] = *covers
                    .entry((face_id, d as u8))
                    .or_insert_with(|| fluid::coverage(shape, d));
            }
            info.push(Info {
                solid: s.solid_render,
                full: s.full_collision,
                permeable: !s.solid_render || s.dampening == 0,
                shade,
                emission: s.emission,
                emissive,
                fluid: FluidState::from_block(state)?,
                model: s.render_shape == RenderShape::Model,
                tints,
                tint_count,
                offset,
                legacy_solid: s.legacy_solid,
                overlay: s.has_class("HalfTransparentBlock") || s.has_class("LeavesBlock"),
                blocks_flow: assets.blocks_fluid_flow.contains(&*s.name),
                ice: s.has_class("IceBlock"),
                sturdy: s.sturdy_full,
                face,
                cover,
            });
        }
        Ok(Self {
            info,
            visibility,
            water_still: layer("minecraft:block/water_still")?,
            water_flow: layer("minecraft:block/water_flow")?,
            water_overlay: layer("minecraft:block/water_overlay")?,
            lava_still: layer("minecraft:block/lava_still")?,
            lava_flow: layer("minecraft:block/lava_flow")?,
        })
    }
    pub fn facts(&self, state: BlockStateId) -> Option<MeshFacts> {
        let i = self.info.get(state.0 as usize)?;
        Some(MeshFacts {
            shade: i.shade,
            emissive: i.emissive,
            legacy_solid: i.legacy_solid,
            light_permeable: i.permeable,
            solid_render: i.solid,
            full_collision: i.full,
            emission: i.emission,
            overlay: i.overlay,
            tint_sources: i.tint_count,
        })
    }
    /// `BlockState.getOffset` for blocks with a random model offset.
    pub fn offset(&self, state: BlockStateId, x: i32, z: i32) -> Option<[f64; 3]> {
        offset(self.info.get(state.0 as usize)?, x, z)
    }
}
/// `OffsetType.XZ`/`XYZ`: a per-column jitter from `Mth.getSeed(x, 0, z)`.
fn offset(info: &Info, x: i32, z: i32) -> Option<[f64; 3]> {
    let (horizontal, vertical) = info.offset?;
    let seed = position_seed(BlockPos::new(x, 0, z));
    let nibble = |shift: u32| ((seed >> shift) & 15) as f32 / 15.0;
    let h = f64::from(horizontal);
    Some([
        ((f64::from(nibble(0)) - 0.5) * 0.5).clamp(-h, h),
        (f64::from(nibble(4)) - 1.0) * f64::from(vertical),
        ((f64::from(nibble(8)) - 0.5) * 0.5).clamp(-h, h),
    ])
}
/// Border width: smooth lighting samples up to two blocks out along a face normal.
const B: i32 = 2;
const P: usize = 16 + 2 * B as usize;
#[inline]
fn pi(x: i32, y: i32, z: i32) -> usize {
    ((y + B) as usize * P + (z + B) as usize) * P + (x + B) as usize
}
/// Blocks and light of one section plus a `B`-block border.
struct Padded {
    blocks: Vec<u16>,
    light: Vec<u8>,
}
impl Padded {
    fn new(region: &[[Option<&Chunk>; 3]; 3], sy: i32) -> Self {
        let mut blocks = vec![0u16; P * P * P];
        let mut light = vec![0u8; P * P * P];
        let base_y = MIN_Y + sy * 16;
        for y in -B..16 + B {
            let wy = base_y + y;
            for z in -B..16 + B {
                for x in -B..16 + B {
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
        (r / n) << 16 | (g / n) << 8 | (b / n)
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
/// Packs 8-bit RGB into the vertex's RGBA8 (red in the low byte), opaque.
fn pack_rgb([r, g, b]: [u32; 3]) -> u32 {
    0xFF00_0000 | b << 16 | g << 8 | r
}
/// Vanilla light coords (block*16 in the low half, sky*16 in the high half) to
/// the vertex layout (sky*16 in bits 0..8, block*16 in bits 8..16).
fn pack_coords(coords: u32) -> u32 {
    (coords >> 16 & 0xFF) | (coords & 0xFF) << 8
}
pub struct Mesher<'a> {
    /// CardinalLighting.NETHER: brighter up/down faces.
    pub nether_shading: bool,
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
        out.visibility = visibility(&pad, &self.tables.info);
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
                    let state = &info[id as usize];
                    if state.solid
                        && Direction::ALL.iter().all(|d| {
                            let [dx, dy, dz] = d.normal();
                            info[pad.blocks[pi(x + dx, y + dy, z + dz)] as usize].solid
                        })
                    {
                        continue;
                    }
                    if state.fluid.kind() != FluidKind::Empty {
                        let water = colors.water[(z * 16 + x) as usize];
                        let nether = self.nether_shading;
                        fluid::tesselate(self.tables, nether, &pad, water, &mut out, [x, y, z]);
                    }
                    if !state.model {
                        continue;
                    }
                    let world = BlockPos::new(center.x * 16 + x, base_y + y, center.z * 16 + z);
                    let Ok(models) = self.assets.model_catalog.collect(BlockStateId(id.into()), world)
                    else {
                        continue;
                    };
                    // ModelBlockRenderer: the first part decides smooth lighting
                    // for all; light-emitting blocks are always lit flat.
                    let Some((_, first)) = models.first() else {
                        continue;
                    };
                    let ao = state.emission == 0 && first.ambient_occlusion;
                    let origin = match offset(state, world.x, world.z) {
                        Some(o) => [
                            x as f32 + o[0] as f32,
                            y as f32 + o[1] as f32,
                            z as f32 + o[2] as f32,
                        ],
                        None => [x as f32, y as f32, z as f32],
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
                    for (_, model) in &models {
                        for face in &model.faces {
                            if face.quad.cull.is_some_and(|d| !visible[d as usize]) {
                                continue;
                            }
                            self.face(&pad, &colors, &mut out, [x, y, z], origin, face, ao, state);
                        }
                    }
                }
            }
        }
        out
    }
    /// `BlockColors` tint source `index`, or `None` for an untinted (-1) layer.
    fn tint(state: &Info, index: i32, colors: &Colors, x: i32, z: i32) -> Option<u32> {
        if index < 0 || index >= i32::from(state.tint_count) {
            return None;
        }
        let column = (z * 16 + x) as usize;
        Some(match state.tints[(index as usize).min(1)] {
            Tint::None => return None,
            Tint::Grass => colors.grass[column],
            Tint::TallGrassUpper => colors.tall_upper[column],
            Tint::Foliage => colors.foliage[column],
            Tint::DryFoliage => colors.dry[column],
            Tint::Water => colors.water[column],
            Tint::Constant(c) => c,
            Tint::Redstone(p) => redstone_color(p),
            Tint::Stem(age) => {
                let a = u32::from(age);
                (a * 32) << 16 | (255 - a * 8) << 8 | (a * 4)
            }
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn face(
        &self,
        pad: &Padded,
        colors: &Colors,
        out: &mut SectionMesh,
        pos: [i32; 3],
        origin: [f32; 3],
        face: &BakedFace,
        ao: bool,
        state: &Info,
    ) {
        let q = &face.quad;
        let shade_face = q.shade_direction_override.unwrap_or(q.facing);
        let directional = lighting::cardinal(self.nether_shading, shade_face);
        // Grey level per vertex before tinting, and vanilla light coords.
        let (gray, coords) = if ao {
            let info = &self.tables.info;
            let (brightness, coords) =
                lighting::ambient_occlusion(pad, info, state, pos, &q.positions, q.facing);
            let gray = brightness.map(|b| lighting::scale(lighting::gray(b), directional));
            (gray, coords)
        } else {
            let coords = match q.cull {
                Some(d) => lighting::coords(pad, state, lighting::step(pos, d)),
                None => lighting::flat(pad, state, pos, &q.positions, q.facing),
            };
            ([lighting::gray(directional); 4], [coords; 4])
        };
        let tint = Self::tint(state, q.tint_index, colors, pos[0], pos[2]);
        let target = if face.translucent {
            &mut out.translucent
        } else {
            &mut out.solid
        };
        for i in 0..4 {
            let g = gray[i] as u32;
            let rgb = match tint {
                Some(t) => [16, 8, 0].map(|shift| g * (t >> shift & 255) / 255),
                None => [g; 3],
            };
            let p = q.positions[i];
            target.push(Vertex {
                pos: [p[0] + origin[0], p[1] + origin[1], p[2] + origin[2]],
                uv: q.uv[i],
                layer: face.layer,
                color: pack_rgb(rgb),
                light: pack_coords(lighting::with_emission(coords[i], q.light_emission)),
            });
        }
    }
}
/// Snapshot of the 3x3 chunk neighbourhood used by a worker job.
pub type Region = [[Option<Arc<Chunk>>; 3]; 3];
pub fn borrow(region: &Region) -> [[Option<&Chunk>; 3]; 3] {
    std::array::from_fn(|z| std::array::from_fn(|x| region[z][x].as_deref()))
}
