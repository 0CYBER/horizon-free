//! `[displacement]` of Horizon's own shader packs (docs/api/shaders.md): near the camera the
//! solid faces whose textures have LabPBR heights are drawn again as a grid of cells, each
//! corner moved along the face's normal by the pack's `displacement`, while the flat quads
//! leave those pixels to them (`native_material.rs`'s `horizon_displaced_layer`).
//!
//! Each frame the solid quads of the sections around the camera are written out of the arena
//! into one buffer as vertices (copied, or from the compact layout's records by ray tracing's
//! unpacking, `ray_scene::unpack_page`), a compute pass picks the ones the grid draws (opaque, with relief, within
//! the distance, facing the eye and in view) into an indirect draw, and the material's
//! displaced pipeline draws one instance a quad.

use super::surface_setup::ray_query::ray_scene::{RtJob, unpack_page, unpack_pipeline};
use crate::mesher::TerrainVertex;

/// The solid quads the grid can draw a frame, and the sections they come from.
const MAX_QUADS: u64 = 1 << 16;
pub(crate) const MAX_SECTIONS: usize = 64;
/// Bytes of one quad's corners.
const QUAD_BYTES: u64 = 4 * std::mem::size_of::<TerrainVertex>() as u64;
/// How far past the flat face the relief may reach, in blocks, for the view test.
const RELIEF_MARGIN: f32 = 0.5;

/// Which quads the grid takes over, for the flat terrain's pipelines (the material's and the
/// shadow's) and the grid's: `horizon_displace_distance` and the LabPBR normal maps
/// `horizon_pbr_normals` (group 3, binding 12), declared by the module.
pub const RELIEF: &str = "
// `[displacement] distance` (0 without): the flat quads leave the pixels within it on faces
// with relief to the grid.
override horizon_displace_distance: f32 = 0.0;
// The mean LabPBR height of a sprite: its normal map's last mip level (1 without one).
fn horizon_relief_mean(layer: u32) -> f32 {
    if textureDimensions(horizon_pbr_normals).x <= 1u
        || layer >= textureNumLayers(horizon_pbr_normals) {
        return 1.0;
    }
    let top = i32(textureNumLevels(horizon_pbr_normals)) - 1;
    return textureLoad(horizon_pbr_normals, vec2(0i), i32(layer), top).a;
}
// The quads the grid redraws near the camera (`PICK`): opaque, on a texture whose height
// varies.
fn horizon_displaced_layer(layer: u32) -> bool {
    return horizon_displace_distance > 0.0 && (layer & 0x8000u) != 0u
        && horizon_relief_mean(layer & 0x7fffu) < 0.998;
}
";

/// The grid's points (`horizon_displace_point`): one instance a quad the pick chose, its
/// corners read from the copy of the arena, each of its `horizon_displace_cells` squared
/// cells two triangles moved along the face's normal by the pack's `displacement`, measured
/// from the texture's mean height so that the face's edges, pinned there, meet their
/// neighbours whatever their relief. Needs `RELIEF`, the pack's `displacement` and
/// `horizon_material_displace` (its vertex stage).
pub const GRID: &str = "
struct HorizonDisplaceSection {
    origin: vec4<f32>,
    first: u32,
    quads: u32,
    pad0: u32,
    pad1: u32,
}
@group(1) @binding(8) var<storage, read> horizon_displace_vertices: array<u32>;
@group(1) @binding(9) var<storage, read> horizon_displace_picked: array<vec2<u32>>;
@group(1) @binding(10) var<storage, read> horizon_displace_sections: array<HorizonDisplaceSection>;
// The grid's cells along a face's side.
override horizon_displace_cells: u32 = 16u;
// Each cell a flat step with walls to its neighbours (`grid_vertices`), not a smooth surface.
override horizon_displace_steps: bool = false;
struct HorizonDisplaceCorner {
    position: vec3<f32>,
    uv: vec2<f32>,
    layer: u32,
    light: vec2<f32>,
    color: vec4<f32>,
}
// A corner of a quad as `TerrainVertex` holds it (24 bytes); light: sky, block.
fn horizon_displace_corner(quad: u32, i: u32, step: f32) -> HorizonDisplaceCorner {
    let b = (quad * 4u + i) * 6u;
    var c: HorizonDisplaceCorner;
    c.position = vec3(bitcast<f32>(horizon_displace_vertices[b]),
        bitcast<f32>(horizon_displace_vertices[b + 1u]),
        bitcast<f32>(horizon_displace_vertices[b + 2u]));
    let uv = horizon_displace_vertices[b + 3u];
    c.uv = vec2<f32>(vec2(uv & 0xffffu, uv >> 16u)) * step;
    let w = horizon_displace_vertices[b + 4u];
    c.layer = w & 0xffffu;
    c.light = vec2(f32((w >> 16u) & 0xffu), f32(w >> 24u));
    c.color = unpack4x8unorm(horizon_displace_vertices[b + 5u]);
    return c;
}
// The height map at `uv`, filtered between the texels of mip level `lod`.
fn horizon_displace_height(uv: vec2<f32>, layer: u32, lod: i32) -> f32 {
    let size = vec2<i32>(textureDimensions(horizon_pbr_normals, lod));
    let p = clamp(uv, vec2(0.0), vec2(1.0)) * vec2<f32>(size) - 0.5;
    let i = vec2<i32>(floor(p));
    let f = p - floor(p);
    let m = size - 1;
    let l = i32(layer);
    let a = textureLoad(horizon_pbr_normals, clamp(i, vec2(0), m), l, lod).a;
    let b = textureLoad(horizon_pbr_normals, clamp(i + vec2(1, 0), vec2(0), m), l, lod).a;
    let c = textureLoad(horizon_pbr_normals, clamp(i + vec2(0, 1), vec2(0), m), l, lod).a;
    let d = textureLoad(horizon_pbr_normals, clamp(i + vec2(1, 1), vec2(0), m), l, lod).a;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
// The height map's texel at `uv` of mip level `lod`.
fn horizon_displace_texel(uv: vec2<f32>, layer: u32, lod: i32) -> f32 {
    let size = vec2<i32>(textureDimensions(horizon_pbr_normals, lod));
    let i = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2(0), size - 1);
    return textureLoad(horizon_pbr_normals, i, i32(layer), lod).a;
}
struct HorizonDisplacePoint {
    // Where it is drawn, and where it lies on the flat face (both camera-relative).
    position: vec3<f32>,
    base: vec3<f32>,
    uv: vec2<f32>,
    layer: u32,
    light: vec2<f32>,
    color: vec4<f32>,
    // The face's normal, and how much of the relief the point has by its distance.
    normal: vec3<f32>,
    near: f32,
}
// A quad's corners, for the steps.
struct HorizonDisplaceQuad {
    c0: HorizonDisplaceCorner,
    c1: HorizonDisplaceCorner,
    c2: HorizonDisplaceCorner,
    c3: HorizonDisplaceCorner,
    origin: vec3<f32>,
    normal: vec3<f32>,
}
fn horizon_displace_at(q: HorizonDisplaceQuad, s: f32, t: f32) -> vec3<f32> {
    return mix(mix(q.c0.position, q.c1.position, s), mix(q.c3.position, q.c2.position, s), t)
        + q.origin;
}
fn horizon_displace_uv(q: HorizonDisplaceQuad, s: f32, t: f32) -> vec2<f32> {
    return mix(mix(q.c0.uv, q.c1.uv, s), mix(q.c3.uv, q.c2.uv, s), t);
}
// How far cell (`i`, `j`) sinks: nothing outside the face, whose edges keep the mean height so
// that they meet their neighbours.
fn horizon_displace_cell_depth(q: HorizonDisplaceQuad, i: i32, j: i32) -> f32 {
    let g = i32(horizon_displace_cells);
    if i < 0 || j < 0 || i >= g || j >= g {
        return 0.0;
    }
    let s = (f32(i) + 0.5) / f32(g);
    let t = (f32(j) + 0.5) / f32(g);
    let base = horizon_displace_at(q, s, t);
    let uv = horizon_displace_uv(q, s, t);
    let layer = q.c0.layer & 0x7fffu;
    let lods = i32(textureNumLevels(horizon_pbr_normals)) - 1;
    let size = f32(textureDimensions(horizon_pbr_normals).x);
    let lod = clamp(i32(floor(log2(size / f32(g)))), 0, lods);
    let material_id = horizon_sprite_material(layer);
    let height = horizon_displace_texel(uv, layer, lod);
    let mean = horizon_relief_mean(layer);
    let depth = displacement(DisplacementInput(base, q.normal, uv, material_id, height))
        - displacement(DisplacementInput(base, q.normal, uv, material_id, mean));
    let r = horizon_displace_distance;
    return depth * (1.0 - smoothstep(0.75 * r, r - 0.1, length(base)));
}
// Vertex `v` of a stepped grid: per cell, from -1 so that the first walls have a cell before
// them, its flat top and its walls to the next cell along each side. A wall faces the way
// out of the higher cell and takes that cell's texture.
fn horizon_displace_step(v: u32, q: HorizonDisplaceQuad) -> HorizonDisplacePoint {
    let g = horizon_displace_cells;
    let cell = v / 18u;
    let part = (v / 6u) % 3u;
    let i = i32(cell % (g + 1u)) - 1;
    let j = i32(cell / (g + 1u)) - 1;
    let cells = f32(g);
    var k = v % 6u;
    var s = 0.0;
    var t = 0.0;
    var uv_s = 0.0;
    var uv_t = 0.0;
    var depth = 0.0;
    if part == 0u {
        if i >= 0 && j >= 0 {
            s = (f32(i) + f32((0x16u >> k) & 1u)) / cells;
            t = (f32(j) + f32((0x34u >> k) & 1u)) / cells;
            depth = horizon_displace_cell_depth(q, i, j);
        }
        uv_s = s;
        uv_t = t;
    } else if part == 1u {
        if j >= 0 {
            let here = horizon_displace_cell_depth(q, i, j);
            let next = horizon_displace_cell_depth(q, i + 1, j);
            if next < here {
                k = (0x453120u >> (k * 4u)) & 15u;
            }
            s = f32(i + 1) / cells;
            t = (f32(j) + f32((0x16u >> k) & 1u)) / cells;
            depth = select(max(here, next), min(here, next), ((0x34u >> k) & 1u) == 1u);
            uv_s = s + select(0.25, -0.25, here <= next) / cells;
            uv_t = t;
        }
    } else {
        if i >= 0 {
            let here = horizon_displace_cell_depth(q, i, j);
            let next = horizon_displace_cell_depth(q, i, j + 1);
            if here < next {
                k = (0x453120u >> (k * 4u)) & 15u;
            }
            s = (f32(i) + f32((0x16u >> k) & 1u)) / cells;
            t = f32(j + 1) / cells;
            depth = select(max(here, next), min(here, next), ((0x34u >> k) & 1u) == 1u);
            uv_s = s;
            uv_t = t + select(0.25, -0.25, here <= next) / cells;
        }
    }
    var p: HorizonDisplacePoint;
    let position = horizon_displace_at(q, s, t);
    let c = clamp(vec2(uv_s, uv_t), vec2(0.0), vec2(1.0));
    p.uv = horizon_displace_uv(q, c.x, c.y);
    p.light = mix(mix(q.c0.light, q.c1.light, s), mix(q.c3.light, q.c2.light, s), t);
    p.color = mix(mix(q.c0.color, q.c1.color, s), mix(q.c3.color, q.c2.color, s), t);
    p.layer = q.c0.layer;
    p.normal = q.normal;
    p.base = horizon_material_displace(position, p.uv, q.c0.layer, p.color,
        vec2<u32>(p.light + 0.5));
    p.near = 1.0 - smoothstep(0.75 * horizon_displace_distance, horizon_displace_distance - 0.1,
        length(p.base));
    p.position = p.base - p.normal * depth;
    return p;
}
// Vertex `v` of instance `instance`'s grid.
fn horizon_displace_point(v: u32, instance: u32) -> HorizonDisplacePoint {
    let pick = horizon_displace_picked[instance];
    let section = horizon_displace_sections[pick.y];
    let c0 = horizon_displace_corner(pick.x, 0u, section.origin.w);
    let c1 = horizon_displace_corner(pick.x, 1u, section.origin.w);
    let c2 = horizon_displace_corner(pick.x, 2u, section.origin.w);
    let c3 = horizon_displace_corner(pick.x, 3u, section.origin.w);
    if horizon_displace_steps {
        return horizon_displace_step(v, HorizonDisplaceQuad(c0, c1, c2, c3, section.origin.xyz,
            normalize(cross(c1.position - c0.position, c2.position - c0.position))));
    }
    // The cell's two triangles wind as the quad's (0, 1, 2) and (0, 2, 3).
    let g = horizon_displace_cells;
    let cell = v / 6u;
    let k = v % 6u;
    let s = f32(cell % g + ((0x16u >> k) & 1u)) / f32(g);
    let t = f32(cell / g + ((0x34u >> k) & 1u)) / f32(g);
    var p: HorizonDisplacePoint;
    let position = mix(mix(c0.position, c1.position, s), mix(c3.position, c2.position, s), t);
    p.uv = mix(mix(c0.uv, c1.uv, s), mix(c3.uv, c2.uv, s), t);
    p.light = mix(mix(c0.light, c1.light, s), mix(c3.light, c2.light, s), t);
    p.color = mix(mix(c0.color, c1.color, s), mix(c3.color, c2.color, s), t);
    p.layer = c0.layer;
    p.normal = normalize(cross(c1.position - c0.position, c2.position - c0.position));
    p.base = horizon_material_displace(position + section.origin.xyz, p.uv, c0.layer, p.color,
        vec2<u32>(p.light + 0.5));
    let layer = c0.layer & 0x7fffu;
    // The heights at about a texel a cell.
    let lods = i32(textureNumLevels(horizon_pbr_normals)) - 1;
    let size = f32(textureDimensions(horizon_pbr_normals).x);
    let lod = clamp(i32(floor(log2(size / f32(g)))), 0, lods);
    let material_id = horizon_sprite_material(layer);
    let height = horizon_displace_height(p.uv, layer, lod);
    let mean = horizon_relief_mean(layer);
    let depth = displacement(DisplacementInput(p.base, p.normal, p.uv, material_id, height))
        - displacement(DisplacementInput(p.base, p.normal, p.uv, material_id, mean));
    // Pinned at the face's edges, easing out toward the distance.
    let edge = min(min(s, 1.0 - s), min(t, 1.0 - t)) * f32(g);
    let r = horizon_displace_distance;
    p.near = 1.0 - smoothstep(0.75 * r, r - 0.1, length(p.base));
    p.position = p.base - p.normal * depth * clamp(edge, 0.0, 1.0) * p.near;
    return p;
}
";

const PICK: &str = "
struct Params {
    view_proj: mat4x4<f32>,
    distance: f32,
    sections: u32,
    margin: f32,
    capacity: u32,
    // 1 for the shadow map's list: every side, wherever the eye looks.
    shadow: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}
struct Section {
    origin: vec4<f32>,
    first: u32,
    quads: u32,
    pad0: u32,
    pad1: u32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> sections: array<Section>;
@group(0) @binding(2) var<storage, read> vertices: array<u32>;
@group(0) @binding(3) var<storage, read_write> picked: array<vec2<u32>>;
@group(0) @binding(4) var<storage, read_write> draw: array<atomic<u32>, 4>;
@group(0) @binding(5) var normals: texture_2d_array<f32>;

fn corner(quad: u32, i: u32, origin: vec3<f32>) -> vec3<f32> {
    let b = (quad * 4u + i) * 6u;
    return vec3(bitcast<f32>(vertices[b]), bitcast<f32>(vertices[b + 1u]),
        bitcast<f32>(vertices[b + 2u])) + origin;
}

// Every corner of the box beyond one side of the view.
fn outside(lo: vec3<f32>, hi: vec3<f32>) -> bool {
    var out = vec4(true);
    var behind = true;
    for (var i = 0u; i < 8u; i++) {
        let p = select(lo, hi, vec3((i & 1u) != 0u, (i & 2u) != 0u, (i & 4u) != 0u));
        let c = params.view_proj * vec4(p, 1.0);
        // Per component: FXC (DirectX 12) drops the whole test for the same `out & vec4(...)`
        // and rejects every quad (#16168).
        out = vec4(out.x && c.x < -c.w, out.y && c.x > c.w,
            out.z && c.y < -c.w, out.w && c.y > c.w);
        behind = behind && c.w <= 0.0;
    }
    return any(out) || behind;
}

@compute @workgroup_size(64) fn pick(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.y >= params.sections {
        return;
    }
    let section = sections[id.y];
    if id.x >= section.quads {
        return;
    }
    let quad = section.first + id.x;
    // As `horizon_displaced_layer`: opaque, with a LabPBR height that varies.
    let layer = vertices[quad * 24u + 4u] & 0xffffu;
    let l = layer & 0x7fffu;
    if (layer & 0x8000u) == 0u || textureDimensions(normals).x <= 1u
        || l >= textureNumLayers(normals) {
        return;
    }
    let top = i32(textureNumLevels(normals)) - 1;
    if textureLoad(normals, vec2(0i), i32(l), top).a >= 0.998 {
        return;
    }
    let p0 = corner(quad, 0u, section.origin.xyz);
    let p1 = corner(quad, 1u, section.origin.xyz);
    let p2 = corner(quad, 2u, section.origin.xyz);
    let p3 = corner(quad, 3u, section.origin.xyz);
    let lo = min(min(p0, p1), min(p2, p3));
    let hi = max(max(p0, p1), max(p2, p3));
    if length(clamp(vec3(0.0), lo, hi)) >= params.distance {
        return;
    }
    // Facing away from the eye (at the origin), where the flat quad is culled too, or out of
    // its view; the shadow map takes them all, as the flat terrain draws both sides there.
    if params.shadow == 0u && (dot(cross(p1 - p0, p2 - p0), p0) >= 0.0
        || outside(lo - params.margin, hi + params.margin)) {
        return;
    }
    let i = atomicAdd(&draw[1], 1u);
    if i >= params.capacity {
        atomicSub(&draw[1], 1u);
        return;
    }
    picked[i] = vec2(quad, id.y);
}
";

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    view_proj: [[f32; 4]; 4],
    distance: f32,
    sections: u32,
    margin: f32,
    capacity: u32,
    shadow: u32,
    pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Section {
    origin: [f32; 4],
    first: u32,
    quads: u32,
    pad: [u32; 2],
}

/// A section the grid may draw from this frame: its arena page, the base vertex of its solid
/// draws (`SectionGpu::solid_base`: in the compact layout past `PACKED_BASE`), its solid quads
/// and its camera-relative origin.
pub struct Near<'a> {
    pub buffer: &'a wgpu::Buffer,
    pub base: u32,
    pub quads: u32,
    pub origin: [f32; 3],
}

/// One list of picked quads and its indirect draw.
pub struct Picks {
    /// The grid's cells along a face's side.
    pub cells: u32,
    /// Each cell a flat step with walls.
    pub steps: bool,
    params: wgpu::Buffer,
    /// The indirect draw: the grid's vertices, then the quads picked.
    draw: wgpu::Buffer,
    pick_group: wgpu::BindGroup,
    /// Group 1 of the pipelines drawing it (bindings 8 to 10).
    pub draw_group: wgpu::BindGroup,
}

pub struct Displacement {
    /// `[displacement] distance`, in blocks.
    pub distance: f32,
    vertices: wgpu::Buffer,
    sections: wgpu::Buffer,
    pick: wgpu::ComputePipeline,
    /// Writes compact sections' quads out as vertices, made on the first one.
    unpack: Option<wgpu::ComputePipeline>,
    /// The quads the eye sees, and the ones the shadow map draws (when the pack has one).
    pub view: Picks,
    pub shadow: Option<Picks>,
    /// Group 1's layout for the grid's pipelines.
    pub draw_layout: wgpu::BindGroupLayout,
    /// CPU-side section table reused between displacement preparation passes.
    table: Vec<Section>,
    /// Sections the pick ran over this frame (0: nothing to draw).
    pub drawn: usize,
}

fn storage(
    binding: u32,
    read_only: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// The shadow map's grid is coarser: its texels near the camera are a 32nd of a block.
pub const SHADOW_CELLS: u32 = 16;

/// The vertices of one quad's grid: two triangles a cell, or in `steps` also two walls a cell,
/// with a row and a column more for the walls at the face's first edges.
fn grid_vertices(cells: u32, steps: bool) -> u32 {
    if steps {
        (cells + 1) * (cells + 1) * 18
    } else {
        cells * cells * 6
    }
}

impl Displacement {
    /// The grid's buffers and pick pass for `[displacement]` with `distance` and `cells`, in
    /// `steps` or not, reading the LabPBR normal maps `normals`; with a list for the shadow map
    /// when `shadow`.
    pub fn new(
        device: &wgpu::Device,
        distance: f32,
        cells: u32,
        steps: bool,
        normals: &wgpu::TextureView,
        shadow: bool,
    ) -> Self {
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        use wgpu::BufferUsages as U;
        // `COPY_SRC` for the tests, which read the quads back.
        let vertices = buffer(
            "Displacement quads",
            MAX_QUADS * QUAD_BYTES,
            U::STORAGE | U::COPY_DST | U::COPY_SRC,
        );
        let sections = buffer(
            "Displacement sections",
            (MAX_SECTIONS * std::mem::size_of::<Section>()) as u64,
            U::STORAGE | U::COPY_DST,
        );
        let compute = wgpu::ShaderStages::COMPUTE;
        let pick_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Displacement pick"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: compute,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true, compute),
                storage(2, true, compute),
                storage(3, false, compute),
                storage(4, false, compute),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let vertex = wgpu::ShaderStages::VERTEX;
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Displacement draw"),
            entries: &[
                storage(8, true, vertex),
                storage(9, true, vertex),
                storage(10, true, vertex),
            ],
        });
        fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource }
        }
        let picks = |cells: u32| {
            let params = buffer(
                "Displacement parameters",
                std::mem::size_of::<Params>() as u64,
                U::UNIFORM | U::COPY_DST,
            );
            let picked = buffer("Displacement picks", MAX_QUADS * 8, U::STORAGE);
            // `COPY_SRC` for the tests, which read the draw back.
            let draw = buffer(
                "Displacement draw",
                16,
                U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC,
            );
            let pick_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Displacement pick"),
                layout: &pick_layout,
                entries: &[
                    entry(0, params.as_entire_binding()),
                    entry(1, sections.as_entire_binding()),
                    entry(2, vertices.as_entire_binding()),
                    entry(3, picked.as_entire_binding()),
                    entry(4, draw.as_entire_binding()),
                    entry(5, wgpu::BindingResource::TextureView(normals)),
                ],
            });
            let draw_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Displacement draw"),
                layout: &draw_layout,
                entries: &[
                    entry(8, vertices.as_entire_binding()),
                    entry(9, picked.as_entire_binding()),
                    entry(10, sections.as_entire_binding()),
                ],
            });
            Picks {
                cells,
                steps,
                params,
                draw,
                pick_group,
                draw_group,
            }
        };
        let view = picks(cells);
        // A step's flat top has to lie where its shadow was cast from, or it shadows itself in
        // grain; a smooth surface hides the difference and can take fewer cells.
        let shadow = shadow.then(|| {
            picks(if steps {
                cells
            } else {
                cells.min(SHADOW_CELLS)
            })
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Displacement pick"),
            source: wgpu::ShaderSource::Wgsl(PICK.into()),
        });
        let pick = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Displacement pick"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Displacement pick"),
                    bind_group_layouts: &[Some(&pick_layout)],
                    ..Default::default()
                }),
            ),
            module: &module,
            entry_point: Some("pick"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            distance,
            vertices,
            sections,
            pick,
            unpack: None,
            view,
            shadow,
            draw_layout,
            table: Vec::new(),
            drawn: 0,
        }
    }

    /// Writes out the solid quads of `near` (nearest first) and picks this frame's in `encoder`.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        near: &[Near<'_>],
        view_proj: glam::Mat4,
    ) {
        self.table.clear();
        self.table.reserve(near.len().min(MAX_SECTIONS));
        let mut first = 0u64;
        let mut most = 0u32;
        // Compact sections' quads, by page.
        let mut unpacks: Vec<(&wgpu::Buffer, RtJob)> = Vec::new();
        for n in near.iter().take(MAX_SECTIONS) {
            let quads = u64::from(n.quads).min(MAX_QUADS - first);
            if quads == 0 {
                break;
            }
            if n.base >= crate::mesher::packed::PACKED_BASE {
                let job = RtJob {
                    base: n.base,
                    quads: quads as u32,
                    out: (first * 4) as u32,
                    pad: 0,
                };
                unpacks.push((n.buffer, job));
            } else {
                let stride = std::mem::size_of::<TerrainVertex>() as u64;
                encoder.copy_buffer_to_buffer(
                    n.buffer,
                    u64::from(n.base) * stride,
                    &self.vertices,
                    first * QUAD_BYTES,
                    quads * QUAD_BYTES,
                );
            }
            self.table.push(Section {
                origin: [
                    n.origin[0],
                    n.origin[1],
                    n.origin[2],
                    1.0 / crate::mesher::UV_STEPS,
                ],
                first: first as u32,
                quads: quads as u32,
                pad: [0; 2],
            });
            first += quads;
            most = most.max(quads as u32);
        }
        self.drawn = self.table.len();
        if self.table.is_empty() {
            return;
        }
        queue.write_buffer(&self.sections, 0, bytemuck::cast_slice(&self.table));
        let lists = std::iter::once((&self.view, 0)).chain(self.shadow.iter().map(|s| (s, 1)));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Displacement pick"),
            timestamp_writes: None,
        });
        if !unpacks.is_empty() {
            let pipeline = self.unpack.get_or_insert_with(|| unpack_pipeline(device));
            let mut pages: Vec<&wgpu::Buffer> = Vec::new();
            for (page, _) in &unpacks {
                if !pages.contains(page) {
                    pages.push(page);
                }
            }
            for page in pages {
                let jobs: Vec<RtJob> = unpacks
                    .iter()
                    .filter(|(p, _)| *p == page)
                    .map(|(_, job)| *job)
                    .collect();
                unpack_page(device, &mut pass, pipeline, page, &jobs, &self.vertices);
            }
        }
        pass.set_pipeline(&self.pick);
        for (picks, shadow) in lists {
            let params = Params {
                view_proj: view_proj.to_cols_array_2d(),
                distance: self.distance,
                sections: self.table.len() as u32,
                margin: RELIEF_MARGIN,
                capacity: MAX_QUADS as u32,
                shadow,
                pad: [0; 3],
            };
            queue.write_buffer(&picks.params, 0, bytemuck::bytes_of(&params));
            let vertices = grid_vertices(picks.cells, picks.steps);
            queue.write_buffer(&picks.draw, 0, bytemuck::cast_slice(&[vertices, 0, 0, 0]));
            pass.set_bind_group(0, &picks.pick_group, &[]);
            pass.dispatch_workgroups(most.div_ceil(64), self.table.len() as u32, 1);
        }
    }

    /// Draws `picks`' grids with `pipeline`, its other groups bound.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        picks: &Picks,
        pipeline: &wgpu::RenderPipeline,
    ) {
        if self.drawn == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, &picks.draw_group, &[]);
        pass.draw_indirect(&picks.draw, 0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_stepped_grid_has_a_top_and_two_walls_a_cell_and_the_first_edges() {
        assert_eq!(super::grid_vertices(16, false), 16 * 16 * 6);
        assert_eq!(super::grid_vertices(16, true), 17 * 17 * 18);
    }

    #[test]
    fn the_pick_validates() {
        let module = naga::front::wgsl::parse_str(super::PICK)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(super::PICK)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(super::PICK)));
    }

    /// The production pick, on each device here (`test_gpu::each_adapter`), of
    /// one opaque quad with relief, facing the eye one block away with the clip
    /// space the view: drawn by the view and the shadow map; and moved along x
    /// and y, still drawn where it reaches into the view (with the relief's
    /// margin), and by the view not at all wholly past one side. FXC dropped
    /// the view's side test and drew nothing (#16168).
    #[test]
    fn the_pick_takes_the_quads_in_view_on_every_backend() {
        let mut tested = false;
        for (adapter, name) in crate::gfx::test_gpu::each_adapter() {
            let (device, queue) =
                match pollster::block_on(adapter.request_device(&Default::default())) {
                    Ok(device) => device,
                    Err(e) => {
                        eprintln!("no device on {name}: {e}");
                        continue;
                    }
                };
            eprintln!("displacement pick on {} ({name})", adapter.get_info().name);
            let normals = normals(&device, &queue);
            let mut displacement =
                super::Displacement::new(&device, 12.0, 16, false, &normals, true);
            for (at, view) in [
                ([0.0, 0.0], 1),
                ([1.25, 0.0], 1),
                ([0.0, -1.25], 1),
                ([2.25, 0.0], 0),
                ([-2.25, 0.0], 0),
                ([0.0, 2.25], 0),
                ([0.0, -2.25], 0),
            ] {
                let quad = quad(&device, at);
                let near = [super::Near {
                    buffer: &quad,
                    base: 0,
                    quads: 1,
                    origin: [0.0; 3],
                }];
                let mut encoder = device.create_command_encoder(&Default::default());
                displacement.prepare(&device, &queue, &mut encoder, &near, glam::Mat4::IDENTITY);
                queue.submit([encoder.finish()]);
                let shadow = &displacement.shadow.as_ref().unwrap().draw;
                assert_eq!(
                    read(&device, &queue, &displacement.view.draw),
                    [16 * 16 * 6, view, 0, 0],
                    "view at {at:?} on {name}"
                );
                assert_eq!(
                    read(&device, &queue, shadow),
                    [16 * 16 * 6, 1, 0, 0],
                    "shadow at {at:?} on {name}"
                );
            }
            tested = true;
        }
        if !tested {
            eprintln!("no GPU: skipped");
        }
    }

    /// The grid's quads, on each device here: those of sections in the compact layout (records
    /// and a quad spilled to its own vertices, on two pages) are written out as the very words
    /// of their vertices, as a wide section's are copied (#16127).
    #[test]
    fn compact_sections_give_the_grid_the_words_of_their_vertices_on_every_backend() {
        use crate::mesher::{TerrainVertex, packed};
        // Whole faces of blocks on the grid, each its own layer, grey level and light: records.
        let face = |i: u32| {
            let (x, z) = ((i % 8) as f32, (i / 8) as f32);
            let gray = 120 + 9 * i;
            [[0, 0], [0, 1], [1, 1], [1, 0]].map(|[dx, dz]: [u32; 2]| TerrainVertex {
                pos: [x + dx as f32, 9.0, z + dz as f32],
                uv: [((dx * 16) << 10) as u16, ((dz * 16) << 10) as u16],
                layer: crate::mesher::OPAQUE_LAYER | i as u16,
                light: ((4 * i) | (60 - 4 * i) << 8) as u16,
                color: gray | gray << 8 | gray << 16 | 0xFF00_0000,
            })
        };
        let section = |quads: std::ops::Range<u32>, slab: bool| {
            let mut v: Vec<TerrainVertex> = quads.flat_map(face).collect();
            if slab {
                // Half a block up: no record, its four vertices after the records.
                let mut half = face(13);
                for c in &mut half {
                    c.pos[1] = 8.5;
                }
                v.extend(half);
            }
            v
        };
        let (a, b, c) = (
            section(0..6, true),
            section(6..9, false),
            section(9..13, true),
        );
        let compact = |v: &[TerrainVertex]| {
            let quads = v.len() / 4;
            let layout = packed::layout(v, &vec![packed::WHITE; quads]);
            assert_eq!(packed::packed_quads(&layout, quads), quads - 1);
            layout
        };
        // Page 1: section a's records from slot 8, c's after them; page 2: b's vertices from 4.
        let mut one = vec![[0u32; 6]; 8];
        one.extend(compact(&a));
        let c_at = one.len() as u32;
        one.extend(compact(&c));
        let mut two = vec![[0u32; 6]; 4];
        two.extend(
            b.iter()
                .map(|v| bytemuck::cast::<TerrainVertex, [u32; 6]>(*v)),
        );
        let base = |slot: u32| packed::PACKED_BASE + 4 * slot;
        let expected: Vec<TerrainVertex> = [&a, &b, &c].into_iter().flatten().copied().collect();
        let mut tested = false;
        for (adapter, name) in crate::gfx::test_gpu::each_adapter() {
            let (device, queue) =
                match pollster::block_on(adapter.request_device(&Default::default())) {
                    Ok(device) => device,
                    Err(e) => {
                        eprintln!("no device on {name}: {e}");
                        continue;
                    }
                };
            eprintln!("displacement quads on {} ({name})", adapter.get_info().name);
            let page = |words: &[[u32; 6]]| {
                use wgpu::util::DeviceExt;
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(words),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                })
            };
            let (one, two) = (page(&one), page(&two));
            let normals = normals(&device, &queue);
            let mut displacement =
                super::Displacement::new(&device, 12.0, 16, false, &normals, false);
            let near = |buffer, base, v: &[TerrainVertex]| super::Near {
                buffer,
                base,
                quads: v.len() as u32 / 4,
                origin: [0.0; 3],
            };
            let near = [
                near(&one, base(8), &a),
                near(&two, 4, &b),
                near(&one, base(c_at), &c),
            ];
            let mut encoder = device.create_command_encoder(&Default::default());
            displacement.prepare(&device, &queue, &mut encoder, &near, glam::Mat4::IDENTITY);
            queue.submit([encoder.finish()]);
            let bytes = std::mem::size_of_val(expected.as_slice()) as u64;
            let got = read_bytes(&device, &queue, &displacement.vertices.slice(..bytes));
            let got: Vec<TerrainVertex> = bytemuck::pod_collect_to_vec(&got);
            assert_eq!(got.len(), expected.len());
            for (i, (got, want)) in got.iter().zip(&expected).enumerate() {
                assert_eq!(
                    bytemuck::bytes_of(got),
                    bytemuck::bytes_of(want),
                    "vertex {i} on {name}"
                );
            }
            tested = true;
        }
        if !tested {
            eprintln!("no GPU: skipped");
        }
    }

    /// One 2x2 layer of LabPBR normals, every height 128/255.
    fn normals(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        use wgpu::util::DeviceExt;
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            Default::default(),
            &[128, 128, 255, 128].repeat(4),
        );
        texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        })
    }

    /// An opaque quad on layer 0 a block wide at z = -1 around `at`, wound
    /// to face the origin.
    fn quad(device: &wgpu::Device, at: [f32; 2]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let corner = |x: f32, y: f32| crate::mesher::TerrainVertex {
            pos: [at[0] + x, at[1] + y, -1.0],
            uv: [0; 2],
            layer: crate::mesher::OPAQUE_LAYER,
            light: 0,
            color: u32::MAX,
        };
        let corners = [
            corner(-0.5, -0.5),
            corner(0.5, -0.5),
            corner(0.5, 0.5),
            corner(-0.5, 0.5),
        ];
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&corners),
            usage: wgpu::BufferUsages::COPY_SRC,
        })
    }

    /// An indirect draw's arguments.
    fn read(device: &wgpu::Device, queue: &wgpu::Queue, draw: &wgpu::Buffer) -> [u32; 4] {
        bytemuck::pod_read_unaligned(&read_bytes(device, queue, &draw.slice(..)))
    }

    fn read_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slice: &wgpu::BufferSlice<'_>,
    ) -> Vec<u8> {
        let (tx, rx) = std::sync::mpsc::channel();
        wgpu::util::DownloadBuffer::read_buffer(device, queue, slice, move |result| {
            tx.send(result.unwrap().to_vec()).unwrap()
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap()
    }
}
