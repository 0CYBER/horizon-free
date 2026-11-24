//! The stages a pack can fill, each a WGSL function with a fixed signature
//! that Horizon calls inside its own passes, and the WGSL put around a
//! pack's files to build a stage: the prelude before (the inputs and the
//! resources), the entry points after.
use crate::inputs::{
    COMPUTE_INPUT, DISPLACEMENT_INPUT, ENTITY_INPUT, Frame, MATERIAL_INPUT, POST_INPUT, SKY_INPUT,
    SURFACE, Struct, TONEMAP_INPUT, VERTEX_INPUT, WEATHER_INPUT, ray_hit,
};
use crate::package::Pack;

/// The bind group of a pack's resources; groups 0 to 2 are the renderer's.
pub const GROUP: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    /// The sky's colour along each view ray, before the sun, moon and stars.
    Sky,
    /// The colour of block surfaces (terrain), before the alpha test and fog.
    Material,
    /// Where each corner of a block's surface is drawn (waving plants, rolling water), for
    /// the world and the shadow map.
    Vertex,
    /// The colour of rain and snow streaks.
    Weather,
    /// A full-screen pass over the world's picture, under the hand and HUD.
    Post,
    /// The picture for an output (the display), once per pixel of it after the post chain,
    /// before the camera overlays and the HUD.
    Tonemap,
    /// The colour of surfaces that are no block terrain (entities, block entities, items,
    /// particles), before the alpha test, the glint and fog. The game draws entities and
    /// block entities through it so far.
    Entity,
    /// 0.2: a dispatch in the post chain (`[[post]] compute = true`), which reads what a post
    /// pass does. Not in `ALL`, since each pass's entry is its own.
    Compute,
    /// 0.2: a dispatch before the world draws (`[[before_world]]`).
    BeforeWorld,
}

/// A resource a stage reads, declared by its prelude.
#[derive(Debug)]
pub struct Resource {
    pub binding: u32,
    pub name: &'static str,
    pub ty: &'static str,
    pub doc: &'static str,
}

const FRAME: Resource = Resource {
    binding: 0,
    name: "frame",
    ty: "var<uniform> frame: Frame",
    doc: "The frame (`Frame`).",
};

/// The shadow map at `binding`.
const fn shadow_map(binding: u32) -> Resource {
    Resource {
        binding,
        name: "shadow_map",
        ty: "var shadow_map: texture_depth_2d",
        doc: "This frame's shadow map: the depth of the solid terrain and entities the light sees, 0 at the light, 1 away from it (and where nothing is), stored at `shadow_position`'s mapping. Valid while `frame.has_shadow` is 1 (the pack asks for it with `[shadow]`), else a 1×1 map that lights everything.",
    }
}

/// The shadow map's comparison sampler at `binding`.
const fn shadow_sampler(binding: u32) -> Resource {
    Resource {
        binding,
        name: "shadow_sampler",
        ty: "var shadow_sampler: sampler_comparison",
        doc: "Depth comparison, bilinear (2×2 percentage-closer filtering): `textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z)` is 1 where the point is lit, 0 in shadow.",
    }
}

/// The shadow map with translucent terrain at `binding`.
const fn shadow_map_translucent(binding: u32) -> Resource {
    Resource {
        binding,
        name: "shadow_map_translucent",
        ty: "var shadow_map_translucent: texture_depth_2d",
        doc: "`shadow_map` with the translucent terrain (water, stained glass, ice) the light passes through drawn in too, read with `shadow_sampler` the same way: a point lit in `shadow_map` but not here lies under such a surface, which `shadow_color` gives the colour of.",
    }
}

/// The colour of the translucent terrain in the shadow map at `binding`.
const fn shadow_color(binding: u32) -> Resource {
    Resource {
        binding,
        name: "shadow_color",
        ty: "var shadow_color: texture_2d<f32>",
        doc: "The texture colour times the vertex colour (rgb: the tint, the face's directional shade and ambient occlusion) of the nearest translucent surface the light passes through at each texel of `shadow_map_translucent`, white where there is none; alpha is 1 on a fluid (water) and the texture's alpha, at most 0.99, on anything else. Read it with `textureLoad` at `shadow_position`'s xy times `textureDimensions(shadow_color)`.",
    }
}

/// A function the material and post preludes give packs.
pub const SHADOW_HELPERS: &str = "\
// The shadow map's distortion toward its centre, of a point at shadow clip `xy`.
fn shadow_distort(xy: vec2<f32>) -> vec2<f32> {
    let k = frame.shadow_distortion;
    return xy / (length(xy) * k + 1.0 - k);
}
// Where the camera-relative point `position` is stored in the shadow map: xy its 0..1 texture
// coordinate (from the top left), z its depth from the light, for `textureSampleCompareLevel`.
fn shadow_position(position: vec3<f32>) -> vec3<f32> {
    let clip = frame.shadow_view_proj * vec4<f32>(position, 1.0);
    let xy = shadow_distort(clip.xy);
    // At most 1, the depth where nothing is: past the map's far end a point stays lit.
    return vec3<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5, min(clip.z, 1.0));
}
";

const LINEAR_SAMPLER: Resource = Resource {
    binding: 4,
    name: "linear_sampler",
    ty: "var linear_sampler: sampler",
    doc: "Bilinear filtering, clamped to the edges.",
};

const NEAREST_SAMPLER: Resource = Resource {
    binding: 5,
    name: "nearest_sampler",
    ty: "var nearest_sampler: sampler",
    doc: "Nearest texel, clamped to the edges.",
};

/// The tonemap stage's resources, at the post stage's bindings for the same names.
const TONEMAP_RESOURCES: [Resource; 4] = [
    FRAME,
    Resource {
        binding: 3,
        name: "previous",
        ty: "var previous: texture_2d<f32>",
        doc: "The post chain's picture at the window's size (without a post chain, the world's at the world's size: a tonemap that reads it keeps TAA from upscaling the world), linear light: what `input.color` samples, for a curve that reads around the pixel.",
    },
    LINEAR_SAMPLER,
    NEAREST_SAMPLER,
];

/// The `[[before_world]]` stage's resources: what is ready before the world draws.
const BEFORE_WORLD_RESOURCES: [Resource; 7] = [
    FRAME,
    LINEAR_SAMPLER,
    NEAREST_SAMPLER,
    shadow_map(6),
    shadow_sampler(7),
    shadow_map_translucent(9),
    shadow_color(10),
];

/// The colour boundary, in every prelude and in mod pipelines' generated entries:
/// vanilla's 2.2 power both ways.
pub const COLOR_HELPERS: &str = "\
// Negatives and NaN made 0 (a comparison with NaN is false), so no `pow` sees them.
fn horizon_sanitize(c: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0), c, c > vec3<f32>(0.0));
}
fn horizon_decode(c: vec3<f32>) -> vec3<f32> {
    return pow(horizon_sanitize(c), vec3<f32>(2.2));
}
fn horizon_encode(c: vec3<f32>) -> vec3<f32> {
    return pow(horizon_sanitize(c), vec3<f32>(1.0 / 2.2));
}
// Alpha is coverage: it stays as it is.
fn horizon_decode_rgba(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(horizon_decode(c.rgb), c.a);
}
fn horizon_encode_rgba(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(horizon_encode(c.rgb), c.a);
}
";

/// Vanilla's fog, in every stage's prelude of a pack with `[frame] fog = "pack"`: the factor
/// terrain.wgsl's `apply_fog` mixes the fog colour by, from the frame's two pairs of distances.
const FOG_HELPERS: &str = "\
// How much of `frame.fog_color` vanilla mixes over a surface at the camera-relative `position`,
// 0..1: the larger of the environment's fog along the straight line and the render distance's
// along the larger of the horizontal distance and the height difference, each 0 before its
// start, 1 from its end and linear between.
fn vanilla_fog(position: vec3<f32>) -> f32 {
    let d = vec2<f32>(length(position), max(length(position.xz), abs(position.y)));
    let start = vec2<f32>(frame.fog_environment.x, frame.fog_render.x);
    let end = vec2<f32>(frame.fog_environment.y, frame.fog_render.y);
    let ramp = select((d - start) / (end - start), vec2<f32>(1.0), d >= end);
    let f = select(ramp, vec2<f32>(0.0), d <= start);
    return max(f.x, f.y);
}
";

const POST_RESOURCES: [Resource; 11] = [
    FRAME,
    Resource {
        binding: 1,
        name: "scene_color",
        ty: "var scene_color: texture_2d<f32>",
        doc: "The world's picture before any pass.",
    },
    Resource {
        binding: 2,
        name: "scene_depth",
        ty: "var scene_depth: texture_depth_2d",
        doc: "The world's depth, reversed (1 near, 0 far and sky).",
    },
    Resource {
        binding: 3,
        name: "previous",
        ty: "var previous: texture_2d<f32>",
        doc: "The previous pass's output (`scene_color` for the first pass).",
    },
    LINEAR_SAMPLER,
    NEAREST_SAMPLER,
    shadow_map(6),
    shadow_sampler(7),
    shadow_map_translucent(9),
    shadow_color(10),
    Resource {
        binding: 11,
        name: "scene_surface",
        ty: "var scene_surface: texture_2d<u32>",
        doc: "What the material stage's `set_surface` stored for the solid terrain at each pixel, unfiltered (`textureLoad`; its texels are the bits of floats, so `bitcast<vec4<f32>>` them): read it through the prelude's `surface_at`, which knows its layout and where it no longer shows. Blank without a material stage that calls `set_surface`.",
    },
];

const BLOCK_LIGHT_COLORS: Resource = Resource {
    binding: 9,
    name: "block_light_colors",
    ty: "var block_light_colors: texture_3d<f32>",
    doc: "The colour of the block light around the camera, from the `[[lights]]` of the light sources nearby: rgb their colours averaged by how near each is, over 4; a how strongly they reach the cell. Read it through `block_light_color`.",
};

const BLOCK_LIGHT_GRID: Resource = Resource {
    binding: 10,
    name: "block_light_grid",
    ty: "var<uniform> block_light_grid: vec4<f32>",
    doc: "Where `block_light_colors` lies: xyz its corner, camera-relative, w the side of one of its cells in blocks.",
};

/// The entity stage's resources: the material stage's shadow maps and coloured-light grid, at
/// the material stage's bindings, so one bind group serves both.
const ENTITY_RESOURCES: [Resource; 8] = [
    FRAME,
    Resource {
        binding: 2,
        name: "block_light_sampler",
        ty: "var block_light_sampler: sampler",
        doc: "Bilinear filtering, clamped to the edges: what `block_light_color` reads `block_light_colors` with.",
    },
    shadow_map(3),
    shadow_sampler(4),
    shadow_map_translucent(7),
    shadow_color(8),
    BLOCK_LIGHT_COLORS,
    BLOCK_LIGHT_GRID,
];

/// The material stage's resources: the frame, and the world as it was before translucent
/// terrain drew (for water and glass to see what lies behind them).
const MATERIAL_RESOURCES: [Resource; 11] = [
    FRAME,
    Resource {
        binding: 1,
        name: "reflection",
        ty: "var reflection: texture_2d<f32>",
        doc: "The water mirror: the world as a camera mirrored about the horizontal water plane around the camera (`frame.water_plane`) sees it, at half resolution, mapped like the world's picture, so water samples it at `input.screen`. Solid terrain, cut away below the plane, over the vanilla sky gradient: not the pack's `sky` stage, nor the sun, moon and stars. Drawn only for a pack whose material reads it, while `frame.has_reflection` is 1; else a 1×1 blank.",
    },
    Resource {
        binding: 2,
        name: "reflection_sampler",
        ty: "var reflection_sampler: sampler",
        doc: "Bilinear filtering, clamped to the edges.",
    },
    shadow_map(3),
    shadow_sampler(4),
    shadow_map_translucent(7),
    shadow_color(8),
    Resource {
        binding: 6,
        name: "opaque_depth",
        ty: "var opaque_depth: texture_depth_2d",
        doc: "The world's depth before translucent terrain (reversed, like `scene_depth`), for the translucent pass: `textureLoad(opaque_depth, vec2<i32>(input.screen * frame.resolution), 0)` is what lies behind water or glass. Stale while solid terrain draws.",
    },
    BLOCK_LIGHT_COLORS,
    BLOCK_LIGHT_GRID,
    Resource {
        binding: 11,
        name: "opaque_color",
        ty: "var opaque_color: texture_2d<f32>",
        doc: "The world's picture before translucent terrain (sky, solid terrain, entities), the size of `opaque_depth`, for the translucent pass: what a surface refracts. Read it with `textureLoad`. Stale while solid terrain draws; black when the world draws straight to the window (a pack without `[[post]]` passes, at full resolution).",
    },
];

/// Functions the post prelude gives packs.
const POST_HELPERS: &str = "\
// The world's depth at `uv` (reversed: 0 is the sky).
fn depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(scene_depth));
    let texel = vec2<i32>(clamp(uv * size, vec2<f32>(0.0), size - 1.0));
    return textureLoad(scene_depth, texel, 0);
}
// The camera-relative world position, in blocks, of the point at `uv` and `depth`.
fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let view = frame.projection_inverse * clip;
    return (frame.view_inverse * vec4<f32>(view.xyz / view.w, 0.0)).xyz;
}
";

/// `block_light_color` of the material and entity preludes: the argument is the sampler the
/// stage has for it. The `[[lights]]` colours are decoded.
macro_rules! block_light_color {
    ($sampler:literal) => {
        concat!(
            "\
// The colour of the block light at the camera-relative `position` (rgb, from the `[[lights]]`
// of the sources nearby) and how strongly they reach it (a, 0 where none does). Block light
// keeps its own brightness (`input.light.x`): this is its colour only.
fn block_light_color(position: vec3<f32>) -> vec4<f32> {
    let size = vec3<f32>(textureDimensions(block_light_colors));
    let uvw = (position - block_light_grid.xyz) / (block_light_grid.w * size);
    if any(uvw < vec3<f32>(0.0)) || any(uvw > vec3<f32>(1.0)) {
        return vec4<f32>(0.0);
    }
    let c = textureSampleLevel(block_light_colors, ",
            $sampler,
            ", uvw, 0.0);
    return vec4<f32>(horizon_decode(c.rgb * 4.0), c.a);
}
"
        )
    };
}

/// The entity prelude's helpers: the coloured-light grid, and the normal its entries give.
const ENTITY_HELPERS: &str = concat!(
    block_light_color!("block_light_sampler"),
    "\
// The surface's normal from the derivatives of its position, turned toward the camera.
fn horizon_entity_normal(position: vec3<f32>) -> vec3<f32> {
    let n = normalize(cross(dpdx(position), dpdy(position)));
    return select(n, -n, dot(n, position) > 0.0);
}
"
);

/// What the material entries (the checker's below and the renderer's) share: the
/// surface's normal and tangent from the derivatives of its position and uv. The block texture
/// is decoded; the LabPBR maps are data. Both sums of the tangent carry the sign of the screen's
/// handedness against `normal`, negative for a normal that faces the camera (the framebuffer's y
/// points down).
const MATERIAL_HELPERS: &str = concat!(
    block_light_color!("reflection_sampler"),
    "\
// What the post stage's `surface_at` reads for this point of solid terrain: its `normal` (a
// unit vector in world axes), `smoothness` (0..1, say for reflections) and `data`, a value of
// the pack's own passed through. Unset, the point keeps the face's own normal and 0 for both;
// translucent terrain stores none.
var<private> horizon_surface: vec4<f32>;
fn set_surface(normal: vec3<f32>, smoothness: f32, data: f32) {
    // Octahedral, y up: the upper half folds onto xz, the lower half unfolds around it.
    let n = normal / (abs(normal.x) + abs(normal.y) + abs(normal.z));
    var e = n.xz;
    if n.y < 0.0 {
        e = (1.0 - abs(n.zx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xz >= vec2<f32>(0.0));
    }
    horizon_surface = vec4<f32>(bitcast<f32>(pack2x16snorm(e)), smoothness, data, 1.0);
}
// The point being shaded: its sprite's layer and mip level (the entries set them).
var<private> horizon_block_layer: u32;
var<private> horizon_block_lod: f32;
// The surface's block texture at `uv` (the sprite's space, as `input.uv`), without the tint:
// what `input.albedo` holds at the point itself, readable anywhere on the sprite, for normals
// or detail generated from the texture.
fn block_texture(uv: vec2<f32>) -> vec4<f32> {
    return horizon_decode_rgba(horizon_block_sample(uv, horizon_block_layer, horizon_block_lod));
}
// `block_texture` at mip level `lod` (0 the full-size texture).
fn block_texture_level(uv: vec2<f32>, lod: f32) -> vec4<f32> {
    return horizon_decode_rgba(horizon_block_sample(uv, horizon_block_layer, lod));
}
// The block texture's size, in texels (16 x 16 for vanilla's).
fn block_texture_size() -> vec2<f32> {
    return horizon_block_size();
}
// The mip level `block_texture` reads at this point (0 up close, more far away).
fn block_texture_lod() -> f32 {
    return horizon_block_lod;
}
// LabPBR: whether the resource packs ship normal (`_n`) or specular (`_s`) maps beside their
// block textures. Without them `block_normal_map` and `block_specular_map` give the defaults
// below.
fn block_pbr_maps() -> bool {
    return horizon_block_pbr_present();
}
// The surface's LabPBR normal map at `uv` (the sprite's space, as `input.uv`): rg the normal's
// x and y in the texture's space (x along `input.tangent`, y down the texture; 0..1 for
// -1..1), b ambient occlusion (1 none), a height (1 the top). (0.5, 0.5, 1, 1) where a texture
// has none.
fn block_normal_map(uv: vec2<f32>) -> vec4<f32> {
    return horizon_block_pbr(uv, horizon_block_layer, horizon_block_lod, 0u);
}
// `block_normal_map` at the block texture's mip level `lod` (the same footprint on the surface,
// where the map is smaller than the texture).
fn block_normal_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32> {
    return horizon_block_pbr(uv, horizon_block_layer, lod, 0u);
}
// The surface's LabPBR specular map at `uv`: r perceptual smoothness, g F0 up to 229/255 and
// a metal's id above, b porosity or subsurface scattering, a emission (up to 254/255; 1
// none). (0, 0, 0, 0) where a texture has none.
fn block_specular_map(uv: vec2<f32>) -> vec4<f32> {
    return horizon_block_pbr(uv, horizon_block_layer, horizon_block_lod, 1u);
}
// `block_specular_map` at the block texture's mip level `lod`.
fn block_specular_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32> {
    return horizon_block_pbr(uv, horizon_block_layer, lod, 1u);
}
// The LabPBR maps' size, in texels (they may be smaller than the block's texture).
fn block_pbr_size() -> vec2<f32> {
    return horizon_block_pbr_size();
}
// Block faces are flat: the derivatives of the position give the face's normal, turned
// toward the camera. Far away they lose precision, so a normal within about 3.6 degrees of
// an axis (every full block face) is that axis exactly: no shimmer on distant faces. The
// gentlest slope of flowing water, one level over a block, tilts about 6 degrees.
fn horizon_material_normal(position: vec3<f32>) -> vec3<f32> {
    let n = normalize(cross(dpdx(position), dpdy(position)));
    let a = abs(n);
    let m = max(a.x, max(a.y, a.z));
    let axis = select(select(vec3(0.0, 0.0, sign(n.z)), vec3(0.0, sign(n.y), 0.0), a.y == m),
        vec3(sign(n.x), 0.0, 0.0), a.x == m);
    let flat_n = select(n, axis, m > 0.998);
    return select(flat_n, -flat_n, dot(flat_n, position) > 0.0);
}
// The direction `uv.x` grows along on the surface, and the sign of the one `uv.y` grows
// along against `cross(normal, tangent)`.
fn horizon_material_tangent(position: vec3<f32>, uv: vec2<f32>, normal: vec3<f32>) -> vec4<f32> {
    let dp1 = dpdx(position);
    let dp2 = dpdy(position);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, normal);
    let dp1perp = cross(normal, dp1);
    let side = select(-1.0, 1.0, dot(dp1, dp2perp) >= 0.0);
    let t = (dp2perp * duv1.x + dp1perp * duv2.x) * side;
    let b = (dp2perp * duv1.y + dp1perp * duv2.y) * side;
    if length(t) < 1e-12 {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    let tangent = normalize(t - normal * dot(normal, t));
    return vec4<f32>(tangent, select(-1.0, 1.0, dot(cross(normal, tangent), b) >= 0.0));
}
"
);

/// The post prelude's `surface_at`, for the post helpers' end.
const POST_SURFACE: &str = "\
// The solid terrain's surface at a pixel, as the material stage's `set_surface` left it.
struct SurfaceData {
    // False where the sky, an entity, water, glass or a particle shows instead.
    present: bool,
    // A unit vector in world axes.
    normal: vec3<f32>,
    smoothness: f32,
    // The pack's own value, passed through.
    data: f32,
}
fn surface_at(texel: vec2<i32>) -> SurfaceData {
    let size = vec2<i32>(textureDimensions(scene_surface));
    let t = clamp(texel, vec2<i32>(0), size - 1);
    let s = bitcast<vec4<f32>>(textureLoad(scene_surface, t, 0));
    let depth_size = vec2<i32>(textureDimensions(scene_depth));
    let depth = textureLoad(scene_depth, clamp(texel, vec2<i32>(0), depth_size - 1), 0);
    // The terrain's own depth is stored with it: anything drawn nearer since hides it.
    let present = s.w > 0.0 && abs(s.w - depth) <= s.w * 1e-5;
    let e = unpack2x16snorm(bitcast<u32>(s.x));
    var n = vec3<f32>(e.x, 1.0 - abs(e.x) - abs(e.y), e.y);
    if n.y < 0.0 {
        n = vec3<f32>((1.0 - abs(n.z)) * sign(n.x), n.y, (1.0 - abs(n.x)) * sign(n.z));
    }
    return SurfaceData(present, select(vec3<f32>(0.0), normalize(n), present),
        select(0.0, s.y, present), select(0.0, s.z, present));
}
";

/// What a pack's prelude holds of the scene rays are traced through (`[ray_tracing]`, 0.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tracing {
    /// The pack didn't ask: none of it.
    None,
    /// The pack asked and the scene isn't there (a GPU without ray queries, ray tracing off):
    /// trace functions that miss, and nothing a device without ray queries refuses.
    Stub,
    /// The pack asked and the renderer binds the scene.
    Scene,
}

/// The scene in the post stage's group 3, for a pack with `[ray_tracing]` while the renderer
/// has it: the acceleration structure first.
pub const RAY_SCENE_BINDING: u32 = 48;
/// Every traced section's quads as the mesher's vertices (six words each, four a quad).
pub const RAY_STORE_BINDING: u32 = 49;
/// `horizon_RaySpace`: the camera in the scene's space (three floats), how many layers a sprite
/// array holds (a `u32`), how many of the scene's instances, its first ones, are solid
/// terrain's (a `u32`), where the translucent terrain's end (a `u32`: the entity layer's
/// instance is the next), where the retained block entities' instances start (a `u32`, after
/// the stream's), 1 in the Nether else 0 (a `u32`, for the entity layer's shade), then at byte
/// 32 the first quad of each of the stream instance's three geometries (three `u32`), 48 bytes
/// in all.
pub const RAY_SPACE_BINDING: u32 = 50;
/// Two words per sprite layer: the pack's `[[materials]]` id, then its `[[tags]]` bits.
pub const RAY_LAYERS_BINDING: u32 = 51;
/// The first of the `RAY_SPRITE_ARRAYS` block sprite arrays, the others after.
pub const RAY_SPRITES_BINDING: u32 = 52;
pub const RAY_SPRITE_ARRAYS: u32 = 8;
/// The scene's layers, as `RAY_LAYERS` declares them to a pack: the bit of each in a ray's cull
/// mask, which the renderer gives the layer's instances as their mask.
pub const RAY_SOLID: u32 = 1;
pub const RAY_TRANSLUCENT: u32 = 2;
/// The entities within the scene's reach, those the camera draws and those it doesn't, only for
/// a pack whose files name it (`names_ray_entity`).
pub const RAY_ENTITY: u32 = 4;
/// What the scene holds besides for such a pack: the entity stream's vertices (`Vertex`,
/// eleven words each, four a quad), its texture atlas, and a sampler as the entities'.
pub const RAY_ENTITY_STORE_BINDING: u32 = 60;
pub const RAY_ENTITY_ATLAS_BINDING: u32 = 61;
pub const RAY_ENTITY_SAMPLER_BINDING: u32 = 62;
/// And the block entities the renderer keeps on the GPU (a closed chest), the quads of the
/// layer's, in the stream's layout and atlas: the last of the layer's bindings.
pub const RAY_ENTITY_RETAINED_BINDING: u32 = 63;
/// The light sources around the camera and a grid over them, only for a pack with
/// `[ray_tracing] lights = true`, in the passes whose files name them (`CheckedStage::traces_lights`): one
/// storage buffer of `u32`. `RAY_LIGHT_HEADER` words of header: the sources' count, the grid's
/// cells a side, the side of a cell in blocks as `f32` bits, the most sources a cell lists, the
/// grid's corner camera-relative as three `f32` (word 4, rewritten each frame), then where the
/// cells' counts (word 8) and lists (word 9) start. Then `RAY_LIGHT_WORDS` words a source: its
/// block's centre from the grid's corner and its level, as four `f32`, its `[[lights]]` colour
/// as written as three `f32`, its `[[materials]]` id; then a count a cell, and
/// `RAY_LIGHT_PER_CELL` indices a cell.
pub const RAY_LIGHTS_BINDING: u32 = 80;
pub const RAY_LIGHT_HEADER: u32 = 16;
pub const RAY_LIGHT_WORDS: u32 = 8;
/// The most sources the buffer holds, the nearest the camera first.
pub const RAY_LIGHT_MAX: u32 = 1024;
/// Cells along each side of the grid, and the side of one in blocks: the grid spans 128 blocks.
pub const RAY_LIGHT_GRID: u32 = 16;
pub const RAY_LIGHT_CELL: f32 = 8.0;
/// The most sources one cell lists, the strongest where it is first.
pub const RAY_LIGHT_PER_CELL: u32 = 24;
/// The look of each quad of the terrain store (`RAY_STORE_BINDING`), only in a stage that reads a
/// terrain hit's `tint`, `shade` or `ao`: two words a quad, `mesher::QuadLook`.
pub const RAY_LOOKS_BINDING: u32 = 81;
/// The bit of an entity vertex's layer that puts its quad in the entity layer, above the
/// renderer's kind bits: a quad without it lets every ray through.
pub const RAY_ENTITY_QUAD: u32 = 1 << 28;

/// The directive the scene's prelude starts with: WGSL takes it before any declaration.
const RAY_ENABLE: &str = "enable wgpu_ray_query;\n";

/// `text` as the prelude of a pack that names `RAY_ENTITY` (`entities`) has it, or as every
/// other pack's: a line marked `+|` is only the first's, one marked `-|` only the others'. The
/// marks go, so the others' prelude is the same as before the entity layer, byte for byte.
fn layered(text: &str, entities: bool) -> String {
    let (keep, drop) = if entities { ("+|", "-|") } else { ("-|", "+|") };
    text.split_inclusive('\n')
        .filter(|line| !line.starts_with(drop))
        .map(|line| line.strip_prefix(keep).unwrap_or(line))
        .collect()
}

/// `text` as the prelude of a stage that reads a terrain hit's look (`looks`: `RayHit.tint`,
/// `shade` or `ao`) has it, or as every other stage's: a line marked `?|` is only the
/// first's, one marked `!|` only the others'. As `layered`, the others' is the same as before.
fn looked(text: &str, looks: bool) -> String {
    let (keep, drop) = if looks { ("?|", "!|") } else { ("!|", "?|") };
    text.split_inclusive('\n')
        .filter(|line| !line.starts_with(drop))
        .map(|line| line.strip_prefix(keep).unwrap_or(line))
        .collect()
}

/// A quad look's shade code as its shade (`mesher::SHADES` by code, 1 for none): the body of the
/// material wrapper's `horizon_look_shade` and of the scene's `horizon_ray_look_shade`
/// (`look_shade`), so a hit and the material stage shade a quad alike.
pub const LOOK_SHADE: &str = "    switch code {
        case 1u: { return 0.5; }
        case 2u: { return 0.6; }
        case 3u: { return 0.8; }
        case 4u: { return 0.9; }
        case 6u: { return 0.72; }
        case 7u: { return 0.54; }
        default: { return 1.0; }
    }
";

/// `LOOK_SHADE` as the WGSL function `name(code: u32) -> f32`.
pub fn look_shade(name: &str) -> String {
    format!("fn {name}(code: u32) -> f32 {{\n{LOOK_SHADE}}}\n")
}

/// The terrain quads' looks a stage that reads a terrain hit's look has besides the scene
/// (`looked`), then `look_shade`.
const RAY_LOOKS: &str = "\
// One look a quad of `horizon_ray_store`, at its first corner's index over 4 (`mesher::QuadLook`):
// the tint (bits 0-23) with the shade code (24-26, 0 where the quad has no look) and the face, the
// four corners' ambient occlusion a byte each. Zero past the buffer.
@group(3) @binding(81) var<storage, read> horizon_ray_looks: array<u32>;
fn horizon_ray_look(quad: u32) -> vec2<u32> {
    let first = quad * 2u;
    if first + 1u >= arrayLength(&horizon_ray_looks) {
        return vec2<u32>(0u);
    }
    return vec2<u32>(horizon_ray_looks[first], horizon_ray_looks[first + 1u]);
}
";

/// The scene's layers and the functions that read solid terrain alone, with the scene or without
/// it: `trace_layers` and `trace_any_layers` come before (`RAY_SCENE`, `RAY_STUB`). `layered`.
const RAY_LAYERS: &str = "\
// The scene's layers, to OR together for `trace_layers` and `trace_any_layers`: solid terrain
// (cut-outs too), and what the translucent pass draws (water, stained glass, ice).
const RAY_SOLID: u32 = 1u;
const RAY_TRANSLUCENT: u32 = 2u;
+|// The entities within the scene's reach, in view or not: mobs, players, armour stands, item
+|// frames, vehicles, and block entities (a chest at rest, and one the camera draws opening).
+|const RAY_ENTITY: u32 = 4u;
// The nearest solid surface from the camera-relative `origin` along the unit vector `direction`,
// within `max_distance` blocks; `distance` is negative where there is none.
fn trace(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32) -> RayHit {
    return trace_layers(origin, direction, max_distance, RAY_SOLID);
}
// Whether solid terrain lies within `max_distance` blocks of `origin` along `direction`: a
// shadow ray, which stops at the first surface it finds.
fn trace_any(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32) -> bool {
    return trace_any_layers(origin, direction, max_distance, RAY_SOLID);
}
";

/// `trace_layers` and `trace_any_layers` without the scene: every ray misses. `layered`.
const RAY_STUB: &str = "\
// The nearest surface of `layers` from the camera-relative `origin` along the unit vector
// `direction`, within `max_distance` blocks. The scene isn't there (`frame.has_ray_tracing` is
// 0): a miss.
fn trace_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> RayHit {
    let horizon_shaded = vec4<f32>(0.0);
    return RayHit(-1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec2<f32>(0.0), vec4<f32>(0.0),
        vec4<f32>(0.0), 0.0, 0.0, vec2<f32>(0.0), 0u, 0u, 0.0, 0u, false, false,
-|        vec4<f32>(0.0), 0u@shaded_tint@);
+|        false, vec4<f32>(0.0), 0u@shaded_tint@);
}
// Whether anything of `layers` lies within `max_distance` blocks of `origin` along `direction`.
fn trace_any_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> bool {
    return false;
}
";

/// The LabPBR maps in group 3 at the material stage's bindings 12 and 13, and what reads them:
/// the material prelude's `block_normal_map` and its kin, and every stage's `hit_normal_map`
/// and `hit_specular_map`. The arguments are the stage's sampler and the colour sprite array
/// whose size the maps' mip levels follow (they may be smaller than the colour).
macro_rules! pbr_maps {
    ($sampler:literal, $sprites:literal) => {
        concat!(
            "\
// The resource packs' LabPBR maps, one array each, a blank 1x1 without them.
@group(3) @binding(12) var horizon_pbr_normals: texture_2d_array<f32>;
@group(3) @binding(13) var horizon_pbr_specular: texture_2d_array<f32>;
fn horizon_block_pbr_present() -> bool {
    return textureDimensions(horizon_pbr_normals).x > 1u;
}
fn horizon_block_pbr_size() -> vec2<f32> {
    return vec2<f32>(textureDimensions(horizon_pbr_normals).xy);
}
fn horizon_block_pbr(uv: vec2<f32>, layer: u32, lod: f32, map: u32) -> vec4<f32> {
    if !horizon_block_pbr_present() || layer >= textureNumLayers(horizon_pbr_normals) {
        return select(vec4<f32>(0.5, 0.5, 1.0, 1.0), vec4<f32>(0.0), map == 1u);
    }
    let s = ",
            $sampler,
            ";
    // The maps may be smaller than the colour: the same footprint is fewer of their texels.
    let scale = f32(textureDimensions(",
            $sprites,
            ").x)
        / f32(textureDimensions(horizon_pbr_normals).x);
    let l = max(lod - log2(scale), 0.0);
    if map == 0u {
        return textureSampleLevel(horizon_pbr_normals, s, uv, i32(layer), l);
    }
    return textureSampleLevel(horizon_pbr_specular, s, uv, i32(layer), l);
}
"
        )
    };
}

/// The bindings `pbr_maps!` declares the LabPBR maps at in group 3: normals, specular.
pub const PBR_NORMALS_BINDING: u32 = 12;
pub const PBR_SPECULAR_BINDING: u32 = 13;

/// The terrain's (`native_material.rs`), with its sprite sampler.
pub const PBR_MAPS_TERRAIN: &str =
    pbr_maps!("horizon_terrain_sprite_sampler", "horizon_terrain_sprites");

/// A traced post, compute or before-world pass's, against the scene: its `nearest_sampler` (at
/// full size, the one mip level a hit reads, as the terrain's sampler there) and the scene's
/// first sprite array.
const PBR_MAPS_POST: &str = pbr_maps!("nearest_sampler", "horizon_ray_sprites_0");

/// A traced entity stage's, against the scene: the frame's bilinear `block_light_sampler`, which
/// reads a hit's texel at its centre (`horizon_hit_pbr`) as it is.
const PBR_MAPS_ENTITY: &str = pbr_maps!("block_light_sampler", "horizon_ray_sprites_0");

/// A traced post, entity, compute or before-world pass's `block_pbr_maps`, as the material prelude's.
const POST_PBR: &str = "\
// LabPBR: whether the resource packs ship normal (`_n`) or specular (`_s`) maps beside their
// block textures. Without them `hit_normal_map` and `hit_specular_map` give their defaults.
fn block_pbr_maps() -> bool {
    return horizon_block_pbr_present();
}
";

/// The LabPBR maps at a hit, in every stage that traces, with the scene or without it; an
/// entity has none yet. `layered`.
const RAY_PBR: &str = "\
// One texel of a LabPBR map at a hit, whatever the stage's sampler filters: the centre of the
// texel `hit.uv` is in at the maps' own full size, kept inside the sprite as the hit's albedo.
fn horizon_hit_pbr(hit: RayHit, map: u32) -> vec4<f32> {
    let inside = hit.uv - max(ceil(hit.uv) - 1.0, vec2<f32>(0.0));
    let size = horizon_block_pbr_size();
    let texel = clamp(floor(inside * size), vec2<f32>(0.0), size - 1.0);
    return horizon_block_pbr((texel + 0.5) / size, hit.horizon_sprite, -32.0, map);
}
// The resource packs' LabPBR normal map at a hit: its sprite's texel at `hit.uv`, at full size
// (mip level 0), as `block_normal_map_level(uv, 0.0)` reads a drawn surface's (rg the normal's
// x and y in the texture's space, x along `hit.tangent`, y down the texture; 0..1 for -1..1,
// b ambient occlusion, a height). (0.5, 0.5, 1, 1) where the texture has none, and on a miss.
fn hit_normal_map(hit: RayHit) -> vec4<f32> {
-|    if hit.distance < 0.0 {
+|    if hit.distance < 0.0 || hit.entity {
        return vec4<f32>(0.5, 0.5, 1.0, 1.0);
    }
    return horizon_hit_pbr(hit, 0u);
}
// The LabPBR specular map at a hit, as `block_specular_map_level(uv, 0.0)` (r perceptual
// smoothness, g F0 or a metal's id, b porosity or subsurface scattering, a emission, 1 none).
// (0, 0, 0, 0) where the texture has none, and on a miss.
fn hit_specular_map(hit: RayHit) -> vec4<f32> {
-|    if hit.distance < 0.0 {
+|    if hit.distance < 0.0 || hit.entity {
        return vec4<f32>(0.0);
    }
    return horizon_hit_pbr(hit, 1u);
}
";

/// A terrain or entity hit as `trace_layers` gives it, from the committed intersection `hit` of
/// a ray from `origin` along `direction`: the body `trace_layers` and `trace_step` share, so both
/// build it alike (`RAY_SCENE`, `RAY_STEP_SCENE`). `layered`, `looked`.
macro_rules! ray_hit {
    () => {
        "    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        let horizon_shaded = vec4<f32>(0.0);
        return RayHit(-1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec2<f32>(0.0), vec4<f32>(0.0),
            vec4<f32>(0.0), 0.0, 0.0, vec2<f32>(0.0), 0u, 0u, 0.0, 0u, false, false,
-|            vec4<f32>(0.0), 0u@shaded_tint@);
+|            false, vec4<f32>(0.0), 0u@shaded_tint@);
    }
+|    if hit.instance_index >= horizon_ray_space.translucent_instances {
+|        return horizon_ray_entity_hit(hit, origin, direction);
+|    }
    let t = horizon_ray_triangle(hit.instance_custom_data, hit.primitive_index, hit.barycentrics);
    let a = horizon_ray_corner(t.corners.x);
    let e1 = horizon_ray_corner(t.corners.y) - a;
    let e2 = horizon_ray_corner(t.corners.z) - a;
    var normal = normalize(cross(e1, e2));
    let back = dot(normal, direction) > 0.0;
    if back {
        normal = -normal;
    }
    let words = vec3<u32>(horizon_ray_word(t.corners.x, 4u), horizon_ray_word(t.corners.y, 4u),
        horizon_ray_word(t.corners.z, 4u));
    let layer = words.x & 0x7fffu;
    let uv = horizon_ray_uv(t);
    let color = unpack4x8unorm(horizon_ray_word(t.corners.x, 5u)) * t.weights.x
        + unpack4x8unorm(horizon_ray_word(t.corners.y, 5u)) * t.weights.y
        + unpack4x8unorm(horizon_ray_word(t.corners.z, 5u)) * t.weights.z;
    // The light's 16 steps a level: sky in the low byte, block in the high one.
    let sky = dot(vec3<f32>((words >> vec3<u32>(16u)) & vec3<u32>(0xffu)), t.weights);
    let block = dot(vec3<f32>(words >> vec3<u32>(24u)), t.weights);
    var material = 0u;
    var tags = 0u;
    if layer * 2u + 1u < arrayLength(&horizon_ray_layers) {
        material = horizon_ray_layers[layer * 2u];
        tags = horizon_ray_layers[layer * 2u + 1u];
    }
    // The colour's alpha byte, as the terrain reads it (mesher.rs's `emitting` and `giving_off`):
    // bit 3 clear on a fluid quad, the high nibble 15 minus the block's light level, so the
    // byte 0xFF, which means nothing, gives level 0, as `MaterialInput.emission` reads it.
    let flags = horizon_ray_word(t.corners.x, 5u) >> 24u;
    let fluid_quad = (flags & 8u) == 0u;
    // Water is in the translucent layer and lava in the solid one, as their passes draw them.
    let translucent = hit.instance_index >= horizon_ray_space.solid_instances;
    let fluid = select(0u, select(2u, 1u, translucent), fluid_quad);
    let emission = f32(15u - (flags >> 4u)) / 15.0;
?|    // The quad's look, read as the material stage reads it: the tint flat and apart from the
?|    // shade, the shade from its code, the corners' ambient occlusion blended over the triangle
?|    // and decoded once. Without a look, use the vertex colour, with shade and AO 1.
?|    let look = horizon_ray_look(t.corners.x / 4u);
?|    let has = ((look.x >> 24u) & 7u) != 0u;
?|    let corners = (vec3<u32>(look.y) >> ((t.corners - vec3<u32>(t.corners.x)) * 8u)) & vec3<u32>(255u);
?|    let ao = select(1.0, horizon_decode(vec3<f32>(dot(vec3<f32>(corners) / 255.0, t.weights))).x, has);
?|    let tint = vec4<f32>(horizon_decode(select(color.rgb, unpack4x8unorm(look.x).rgb, has)), 1.0);
?|    let shade = horizon_decode(vec3<f32>(horizon_ray_look_shade((look.x >> 24u) & 7u))).x;
    let horizon_shaded = vec4<f32>(horizon_decode(color.rgb), 1.0);
    return RayHit(hit.t, origin + direction * hit.t, normal, uv,
!|        horizon_decode_rgba(horizon_ray_texel(layer, uv)), horizon_shaded, 1.0, 1.0,
?|        horizon_decode_rgba(horizon_ray_texel(layer, uv)), tint, shade, ao,
        min(vec2<f32>(block, sky) / 240.0, vec2<f32>(1.0)), material, tags, emission, fluid,
-|        translucent, back, horizon_ray_tangent(t, e1, e2, normal), layer@shaded_tint@);
+|        translucent, back, false, horizon_ray_tangent(t, e1, e2, normal), layer@shaded_tint@);
"
    };
}

/// `trace_layers` and `trace_any_layers` through the scene the renderer binds
/// (gfx/rt/scene.rs): a hit is a triangle of a quad in the store, read as terrain.wgsl reads
/// the mesher's vertices, or for a pack that names `RAY_ENTITY` one of the entity stream's
/// (`RAY_ENTITY_SCENE`). `layered`.
const RAY_SCENE: &str = concat!(
    "\
struct horizon_RaySpace {
    // The camera in the scene's space: from the corner of the scene's anchor column, in blocks.
    camera: vec3<f32>,
    // The layers one sprite array holds (terrain.wgsl's `SPRITE_SPLIT`).
    sprite_split: u32,
    // The scene's instances of solid terrain, its first ones: the others are translucent terrain.
    solid_instances: u32,
+|    // Where translucent terrain's instances end: the entity layer's is the next.
+|    translucent_instances: u32,
+|    // The first instance of the block entities kept on the GPU, after the stream's: their quads
+|    // are in `horizon_ray_retained`.
+|    retained_instances: u32,
+|    // 1 in the Nether, where vanilla's entity shade lights from below (entity.wgsl's).
+|    nether: u32,
+|    // The first quad of each of the entity layer's three geometries in `horizon_ray_entities`.
+|    entity_bases: vec3<u32>,
}
@group(3) @binding(48) var horizon_ray_scene: acceleration_structure;
// Every traced section's quads as `TerrainVertex` (six words: the position, the uv's two u16,
// the layer and the light, the RGBA8 colour), four to a quad; an instance's custom data is
// its section's first quad.
@group(3) @binding(49) var<storage, read> horizon_ray_store: array<u32>;
@group(3) @binding(50) var<uniform> horizon_ray_space: horizon_RaySpace;
// Two words a sprite layer: the pack's material id, then its tags.
@group(3) @binding(51) var<storage, read> horizon_ray_layers: array<u32>;
@group(3) @binding(52) var horizon_ray_sprites_0: texture_2d_array<f32>;
@group(3) @binding(53) var horizon_ray_sprites_1: texture_2d_array<f32>;
@group(3) @binding(54) var horizon_ray_sprites_2: texture_2d_array<f32>;
@group(3) @binding(55) var horizon_ray_sprites_3: texture_2d_array<f32>;
@group(3) @binding(56) var horizon_ray_sprites_4: texture_2d_array<f32>;
@group(3) @binding(57) var horizon_ray_sprites_5: texture_2d_array<f32>;
@group(3) @binding(58) var horizon_ray_sprites_6: texture_2d_array<f32>;
@group(3) @binding(59) var horizon_ray_sprites_7: texture_2d_array<f32>;
// A sprite's texel at full size (mip level 0), from whichever array holds `layer`, as stored.
fn horizon_ray_texel(layer: u32, uv: vec2<f32>) -> vec4<f32> {
    let l = i32(layer % horizon_ray_space.sprite_split);
    let array = layer / horizon_ray_space.sprite_split;
    var size: vec2<u32>;
    switch array {
        case 0u: { size = textureDimensions(horizon_ray_sprites_0); }
        case 1u: { size = textureDimensions(horizon_ray_sprites_1); }
        case 2u: { size = textureDimensions(horizon_ray_sprites_2); }
        case 3u: { size = textureDimensions(horizon_ray_sprites_3); }
        case 4u: { size = textureDimensions(horizon_ray_sprites_4); }
        case 5u: { size = textureDimensions(horizon_ray_sprites_5); }
        case 6u: { size = textureDimensions(horizon_ray_sprites_6); }
        default: { size = textureDimensions(horizon_ray_sprites_7); }
    }
    // terrain.wgsl's `tile_uv`: a sprite's own uv, 1 included, stays inside it.
    let inside = uv - max(ceil(uv) - 1.0, vec2<f32>(0.0));
    let t = clamp(vec2<i32>(inside * vec2<f32>(size)), vec2<i32>(0), vec2<i32>(size) - 1);
    switch array {
        case 0u: { return textureLoad(horizon_ray_sprites_0, t, l, 0); }
        case 1u: { return textureLoad(horizon_ray_sprites_1, t, l, 0); }
        case 2u: { return textureLoad(horizon_ray_sprites_2, t, l, 0); }
        case 3u: { return textureLoad(horizon_ray_sprites_3, t, l, 0); }
        case 4u: { return textureLoad(horizon_ray_sprites_4, t, l, 0); }
        case 5u: { return textureLoad(horizon_ray_sprites_5, t, l, 0); }
        case 6u: { return textureLoad(horizon_ray_sprites_6, t, l, 0); }
        default: { return textureLoad(horizon_ray_sprites_7, t, l, 0); }
    }
}
fn horizon_ray_word(vertex: u32, i: u32) -> u32 {
    return horizon_ray_store[vertex * 6u + i];
}
// A corner's position, relative to its section.
fn horizon_ray_corner(vertex: u32) -> vec3<f32> {
    return vec3<f32>(bitcast<f32>(horizon_ray_word(vertex, 0u)),
        bitcast<f32>(horizon_ray_word(vertex, 1u)), bitcast<f32>(horizon_ray_word(vertex, 2u)));
}
// A corner's uv in its sprite: 32768 steps a unit (terrain.wgsl's `terrain_uv`).
fn horizon_ray_corner_uv(vertex: u32) -> vec2<f32> {
    let w = horizon_ray_word(vertex, 3u);
    return vec2<f32>(f32(w & 0xffffu), f32(w >> 16u)) / 32768.0;
}
// A triangle's three corners in the store and the point's weight on each. A quad's
// triangles are its corners (0, 1, 2) and (0, 2, 3).
struct horizon_RayTriangle {
    corners: vec3<u32>,
    weights: vec3<f32>,
}
fn horizon_ray_triangle(quad: u32, primitive: u32, barycentrics: vec2<f32>) -> horizon_RayTriangle {
    let first = (quad + primitive / 2u) * 4u;
    let second = (primitive & 1u) == 1u;
    return horizon_RayTriangle(
        vec3<u32>(first, first + select(1u, 2u, second), first + select(2u, 3u, second)),
        vec3<f32>(1.0 - barycentrics.x - barycentrics.y, barycentrics.x, barycentrics.y));
}
fn horizon_ray_uv(t: horizon_RayTriangle) -> vec2<f32> {
    return horizon_ray_corner_uv(t.corners.x) * t.weights.x
        + horizon_ray_corner_uv(t.corners.y) * t.weights.y
        + horizon_ray_corner_uv(t.corners.z) * t.weights.z;
}
// The direction `uv.x` grows along on a triangle whose edges from its first corner are `e1` and
// `e2`, and the sign of the one `uv.y` grows along against `cross(normal, tangent)`: the
// material stage's `MaterialInput.tangent`, from the corners rather than the derivatives.
fn horizon_ray_tangent(t: horizon_RayTriangle, e1: vec3<f32>, e2: vec3<f32>, normal: vec3<f32>) -> vec4<f32> {
    let uv = horizon_ray_corner_uv(t.corners.x);
    let d1 = horizon_ray_corner_uv(t.corners.y) - uv;
    let d2 = horizon_ray_corner_uv(t.corners.z) - uv;
    // The position's derivatives along u and v, times the uv triangle's area: its sign alone
    // matters here, so it is kept and the length dropped.
    let s = select(-1.0, 1.0, d1.x * d2.y - d2.x * d1.y >= 0.0);
    let du = (e1 * d2.y - e2 * d1.y) * s;
    let dv = (e2 * d1.x - e1 * d2.x) * s;
    let along = du - normal * dot(normal, du);
    if length(along) < 1e-12 {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    let tangent = normalize(along);
    return vec4<f32>(tangent, select(-1.0, 1.0, dot(cross(normal, tangent), dv) >= 0.0));
}
// Whether a triangle stops the ray where it crosses it, as its pass draws the texel there. Solid
// terrain's cutout test: bit 15 of the layer marks a quad over opaque texels; another stops it
// where its texel's alpha is 0.5 or more. Translucent terrain's (terrain.wgsl's
// `fs_translucent`): every texel but a clear one, alpha below 0.01, stops it.
fn horizon_ray_stops(c: RayIntersection) -> bool {
+|    if c.instance_index >= horizon_ray_space.translucent_instances {
+|        return horizon_ray_entity_stops(c);
+|    }
    let t = horizon_ray_triangle(c.instance_custom_data, c.primitive_index, c.barycentrics);
    let layer = horizon_ray_word(t.corners.x, 4u) & 0xffffu;
    if c.instance_index >= horizon_ray_space.solid_instances {
        return horizon_ray_texel(layer & 0x7fffu, horizon_ray_uv(t)).a >= 0.01;
    }
    if (layer & 0x8000u) != 0u {
        return true;
    }
    return horizon_ray_texel(layer, horizon_ray_uv(t)).a >= 0.5;
}
// The nearest triangle of `layers` that stops the ray, or with `first` any of them: an
// instance's mask is its layer's bit.
fn horizon_ray_cast(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32, first: bool) -> RayIntersection {
    var query: ray_query;
    let flags = select(RAY_FLAG_NONE, RAY_FLAG_TERMINATE_ON_FIRST_HIT, first);
-|    rayQueryInitialize(&query, horizon_ray_scene, RayDesc(flags, layers & 3u, 0.0,
+|    rayQueryInitialize(&query, horizon_ray_scene, RayDesc(flags, layers & 7u, 0.0,
        max(max_distance, 0.0), origin + horizon_ray_space.camera, direction));
    while rayQueryProceed(&query) {
        if horizon_ray_stops(rayQueryGetCandidateIntersection(&query)) {
            rayQueryConfirmIntersection(&query);
        }
    }
    return rayQueryGetCommittedIntersection(&query);
}
-|// The nearest surface of `layers` (`RAY_SOLID`, `RAY_TRANSLUCENT` or both) from the
+|// The nearest surface of `layers` (`RAY_SOLID`, `RAY_TRANSLUCENT`, `RAY_ENTITY` ORed) from the
// camera-relative `origin` along the unit vector `direction`, within `max_distance` blocks;
// `distance` is negative where there is none.
fn trace_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> RayHit {
    let hit = horizon_ray_cast(origin, direction, max_distance, layers, false);
",
    ray_hit!(),
    "}
// Whether anything of `layers` lies within `max_distance` blocks of `origin` along `direction`.
fn trace_any_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> bool {
    return horizon_ray_cast(origin, direction, max_distance, layers, true).kind
        != RAY_QUERY_INTERSECTION_NONE;
}
"
);

/// The light sources for a stage whose files name them (`names_lights`), with the scene.
const RAY_LIGHTS_SCENE: &str = "\
// The light sources (blocks that give off light) within 64 blocks or so of the camera, the
// nearest first, and a grid of 8-block cells over them, each listing the sources that reach it.
@group(3) @binding(80) var<storage, read> horizon_ray_lights: array<u32>;
// A light source: the centre of its block, camera-relative, its light level (1 to 15), its
// `[[lights]]` colour, linear (white for a block the pack gives none), and its `[[materials]]` id.
struct RayLight {
    position: vec3<f32>,
    level: f32,
    color: vec3<f32>,
    material: u32,
}
fn horizon_ray_light_float(i: u32) -> f32 {
    return bitcast<f32>(horizon_ray_lights[i]);
}
fn horizon_ray_light_vec(i: u32) -> vec3<f32> {
    return vec3<f32>(horizon_ray_light_float(i), horizon_ray_light_float(i + 1u),
        horizon_ray_light_float(i + 2u));
}
// How many sources `ray_light` reads, 0 to 1024.
fn ray_light_count() -> u32 {
    return horizon_ray_lights[0];
}
// The source `i`, below `ray_light_count()`; level 0 past it (and for `ray_light_at`'s
// 0xffffffffu).
fn ray_light(i: u32) -> RayLight {
    let none = RayLight(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 0u);
    if i >= ray_light_count() {
        return none;
    }
    let at = 16u + i * 8u;
    if at + 7u >= arrayLength(&horizon_ray_lights) {
        return none;
    }
    return RayLight(horizon_ray_light_vec(at) + horizon_ray_light_vec(4u),
        horizon_ray_light_float(at + 3u), horizon_decode(horizon_ray_light_vec(at + 4u)),
        horizon_ray_lights[at + 7u]);
}
// The sources that reach the camera-relative `position`, as the first and the count of the
// indices to read with `ray_light_at`: at most 24, the strongest there first; none outside the
// grid.
fn ray_lights_near(position: vec3<f32>) -> vec2<u32> {
    let n = horizon_ray_lights[1];
    let cell = floor((position - horizon_ray_light_vec(4u)) / horizon_ray_light_float(2));
    if n == 0u || any(cell < vec3<f32>(0.0)) || any(cell >= vec3<f32>(f32(n))) {
        return vec2<u32>(0u);
    }
    let c = vec3<u32>(cell);
    let index = c.x + n * (c.y + n * c.z);
    return vec2<u32>(index * horizon_ray_lights[3], horizon_ray_lights[horizon_ray_lights[8] + index]);
}
// The index, for `ray_light`, of the source `i` of a cell's list (`ray_lights_near`);
// 0xffffffffu, which `ray_light` reads as no source, where `i` is past its cell's count.
fn ray_light_at(i: u32) -> u32 {
    let n = horizon_ray_lights[1];
    let per = max(horizon_ray_lights[3], 1u);
    let counts = horizon_ray_lights[8];
    let lists = horizon_ray_lights[9];
    let length = arrayLength(&horizon_ray_lights);
    let cell = i / per;
    if cell >= n * n * n || counts >= length || cell >= length - counts {
        return 0xffffffffu;
    }
    if i % per >= horizon_ray_lights[counts + cell] || lists >= length || i >= length - lists {
        return 0xffffffffu;
    }
    return horizon_ray_lights[lists + i];
}
";

/// `RAY_LIGHTS_SCENE` without the scene: no sources.
const RAY_LIGHTS_STUB: &str = "\
// A light source. The scene isn't there (`frame.has_ray_tracing` is false): there are none.
struct RayLight {
    position: vec3<f32>,
    level: f32,
    color: vec3<f32>,
    material: u32,
}
fn ray_light_count() -> u32 {
    return 0u;
}
fn ray_light(i: u32) -> RayLight {
    return RayLight(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 0u);
}
fn ray_lights_near(position: vec3<f32>) -> vec2<u32> {
    return vec2<u32>(0u);
}
fn ray_light_at(i: u32) -> u32 {
    return 0xffffffffu;
}
";

/// What `[ray_tracing] step = true` brings a stage whose files name one of `STEP_NAMES`, with the
/// scene or without it: the actions `ray_step` returns. `RAY_STEP_SCENE` or `RAY_STEP_STUB` after.
const RAY_STEP: &str = "\
// What the pack's `ray_step` tells `trace_step` to do with a candidate, bits to OR together: go
// on past it (`RAY_PASS`), keep it as the nearest hit so far, which shortens the ray to it
// (`RAY_STOP`), end the query now (`RAY_END`): the hit is then the nearest kept by now, this
// candidate too where the same answer has `RAY_STOP`.
const RAY_PASS: u32 = 0u;
const RAY_STOP: u32 = 1u;
const RAY_END: u32 = 2u;
";

/// `trace_step` through the scene the renderer binds: one ray query, every candidate handed to
/// the pack's `ray_step` with the pack's `RayState`, which the pack defines after this text
/// (WGSL takes module-scope names in any order). A candidate holds only what the query gives
/// for nothing; the `candidate_` functions read the rest, each as `trace_layers` reads it for a
/// hit, only when the step calls them. `layered`, `looked`.
const RAY_STEP_SCENE: &str = concat!(
    "\
// A surface a ray of `trace_step` crosses, as `ray_step` gets it: its distance along the ray, in
// blocks, and its layer, `RAY_SOLID`, `RAY_TRANSLUCENT` or `RAY_ENTITY`. The `candidate_`
// functions read the rest of it, each only where called.
struct RayCandidate {
    distance: f32,
    layer: u32,
    horizon_intersection: RayIntersection,
}
fn horizon_ray_candidate(i: RayIntersection) -> RayCandidate {
    var layer = RAY_SOLID;
    if i.instance_index >= horizon_ray_space.solid_instances {
        layer = RAY_TRANSLUCENT;
    }
+|    if i.instance_index >= horizon_ray_space.translucent_instances {
+|        layer = RAY_ENTITY;
+|    }
    return RayCandidate(i.t, layer, i);
}
// Whether the candidate stops a ray of `trace_layers`: what its pass draws there.
fn candidate_stops(c: RayCandidate) -> bool {
    return horizon_ray_stops(c.horizon_intersection);
}
fn horizon_candidate_triangle(c: RayCandidate) -> horizon_RayTriangle {
    let i = c.horizon_intersection;
    return horizon_ray_triangle(i.instance_custom_data, i.primitive_index, i.barycentrics);
}
fn horizon_candidate_sprite(t: horizon_RayTriangle) -> u32 {
    return horizon_ray_word(t.corners.x, 4u) & 0x7fffu;
}
// `RayHit.albedo` at the candidate: its texel at full size, linear, with its alpha.
fn candidate_albedo(c: RayCandidate) -> vec4<f32> {
+|    if c.layer == RAY_ENTITY {
+|        let t = horizon_ray_entity_triangle(c.horizon_intersection);
+|        return horizon_decode_rgba(horizon_ray_entity_texel(horizon_ray_entity_mix(t, 3u)));
+|    }
    let t = horizon_candidate_triangle(c);
    return horizon_decode_rgba(horizon_ray_texel(horizon_candidate_sprite(t), horizon_ray_uv(t)));
}
// `RayHit.tint` at the candidate.
fn candidate_tint(c: RayCandidate) -> vec4<f32> {
+|    if c.layer == RAY_ENTITY {
+|        return horizon_decode_rgba(horizon_ray_entity_color(horizon_ray_entity_triangle(c.horizon_intersection)));
+|    }
    let t = horizon_candidate_triangle(c);
    let color = unpack4x8unorm(horizon_ray_word(t.corners.x, 5u)) * t.weights.x
        + unpack4x8unorm(horizon_ray_word(t.corners.y, 5u)) * t.weights.y
        + unpack4x8unorm(horizon_ray_word(t.corners.z, 5u)) * t.weights.z;
?|    let look = horizon_ray_look(t.corners.x / 4u);
?|    let has = ((look.x >> 24u) & 7u) != 0u;
?|    return vec4<f32>(horizon_decode(select(color.rgb, unpack4x8unorm(look.x).rgb, has)), 1.0);
!|    return vec4<f32>(horizon_decode(color.rgb), 1.0);
}
// The candidate's sprite's word `word` of `horizon_ray_layers`: 0 its material, 1 its tags.
fn horizon_candidate_layer(c: RayCandidate, word: u32) -> u32 {
+|    if c.layer == RAY_ENTITY {
+|        return 0u;
+|    }
    let layer = horizon_candidate_sprite(horizon_candidate_triangle(c));
    if layer * 2u + 1u < arrayLength(&horizon_ray_layers) {
        return horizon_ray_layers[layer * 2u + word];
    }
    return 0u;
}
// `RayHit.material` at the candidate.
fn candidate_material(c: RayCandidate) -> u32 {
    return horizon_candidate_layer(c, 0u);
}
// `RayHit.tags` at the candidate.
fn candidate_tags(c: RayCandidate) -> u32 {
    return horizon_candidate_layer(c, 1u);
}
// `RayHit.fluid` at the candidate: 1 water, 2 lava, 0 anything else.
fn candidate_fluid(c: RayCandidate) -> u32 {
+|    if c.layer == RAY_ENTITY {
+|        return 0u;
+|    }
    let flags = horizon_ray_word(horizon_candidate_triangle(c).corners.x, 5u) >> 24u;
    return select(0u, select(2u, 1u, c.layer == RAY_TRANSLUCENT), (flags & 8u) == 0u);
}
// One ray query from the camera-relative `origin` along the unit vector `direction`, within
// `max_distance` blocks, over `layers`: every candidate goes to the pack's `ray_step` with
// `state`, in no set order. The nearest candidate a step kept (`RAY_STOP`) is the hit, as
// `trace_layers` gives it; `distance` is negative where it kept none.
fn trace_step(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32, state: ptr<function, RayState>) -> RayHit {
    var query: ray_query;
-|    rayQueryInitialize(&query, horizon_ray_scene, RayDesc(RAY_FLAG_NONE, layers & 3u, 0.0,
+|    rayQueryInitialize(&query, horizon_ray_scene, RayDesc(RAY_FLAG_NONE, layers & 7u, 0.0,
        max(max_distance, 0.0), origin + horizon_ray_space.camera, direction));
    while rayQueryProceed(&query) {
        let action = ray_step(horizon_ray_candidate(rayQueryGetCandidateIntersection(&query)), state);
        if (action & RAY_STOP) != 0u {
            rayQueryConfirmIntersection(&query);
        }
        if (action & RAY_END) != 0u {
            rayQueryTerminate(&query);
        }
    }
    return horizon_ray_step_hit(rayQueryGetCommittedIntersection(&query), origin, direction);
}
fn horizon_ray_step_hit(hit: RayIntersection, origin: vec3<f32>, direction: vec3<f32>) -> RayHit {
",
    ray_hit!(),
    "}
"
);

/// `RAY_STEP_SCENE` without the scene: no candidate, every ray misses, `state` as it was. A call
/// to `ray_step` that never runs keeps the pack's step in the stage, so the check sees what it
/// reaches (a discard, a look, a trace) as it would against the scene. `layered`.
const RAY_STEP_STUB: &str = "\
// A surface a ray of `trace_step` crosses. The scene isn't there (`frame.has_ray_tracing` is
// 0): there are none.
struct RayCandidate {
    distance: f32,
    layer: u32,
}
fn candidate_stops(c: RayCandidate) -> bool {
    return false;
}
fn candidate_albedo(c: RayCandidate) -> vec4<f32> {
    return vec4<f32>(0.0);
}
fn candidate_tint(c: RayCandidate) -> vec4<f32> {
    return vec4<f32>(0.0);
}
fn candidate_material(c: RayCandidate) -> u32 {
    return 0u;
}
fn candidate_tags(c: RayCandidate) -> u32 {
    return 0u;
}
fn candidate_fluid(c: RayCandidate) -> u32 {
    return 0u;
}
// Every ray misses: `ray_step` sees no candidate and `state` stays as it was.
fn trace_step(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32, state: ptr<function, RayState>) -> RayHit {
    if false {
        _ = ray_step(RayCandidate(-1.0, 0u), state);
    }
    let horizon_shaded = vec4<f32>(0.0);
    return RayHit(-1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec2<f32>(0.0), vec4<f32>(0.0),
        vec4<f32>(0.0), 0.0, 0.0, vec2<f32>(0.0), 0u, 0u, 0.0, 0u, false, false,
-|        vec4<f32>(0.0), 0u@shaded_tint@);
+|        false, vec4<f32>(0.0), 0u@shaded_tint@);
}
";

/// What the scene adds for a pack that names `RAY_ENTITY`: the entity layer, read from the
/// entity stream's vertices as entity.wgsl reads them, its candidates kept as it keeps a pixel.
const RAY_ENTITY_SCENE: &str = "\
// The entity stream's vertices, eleven words each (the position, the uv, the layer, the RGBA8
// colour, the light, the normal, the dissolve mask's uv), four to a quad.
@group(3) @binding(60) var<storage, read> horizon_ray_entities: array<u32>;
@group(3) @binding(61) var horizon_ray_entity_atlas: texture_2d<f32>;
@group(3) @binding(62) var horizon_ray_entity_sampler: sampler;
// The block entities kept on the GPU (a closed chest), the quads of the layer's, in the same
// layout and atlas: a retained instance's custom data is its section's first quad here.
@group(3) @binding(63) var<storage, read> horizon_ray_retained: array<u32>;
// Added to a retained vertex's index: the vertex is in `horizon_ray_retained`.
const horizon_ray_retained_vertex: u32 = 0x80000000u;
fn horizon_ray_entity_word(vertex: u32, i: u32) -> u32 {
    if vertex >= horizon_ray_retained_vertex {
        return horizon_ray_retained[(vertex - horizon_ray_retained_vertex) * 11u + i];
    }
    return horizon_ray_entities[vertex * 11u + i];
}
fn horizon_ray_entity_corner(vertex: u32) -> vec3<f32> {
    return vec3<f32>(bitcast<f32>(horizon_ray_entity_word(vertex, 0u)),
        bitcast<f32>(horizon_ray_entity_word(vertex, 1u)), bitcast<f32>(horizon_ray_entity_word(vertex, 2u)));
}
// A corner's two floats from word `i` on: its uv (3), its dissolve mask's (9).
fn horizon_ray_entity_pair(vertex: u32, i: u32) -> vec2<f32> {
    return vec2<f32>(bitcast<f32>(horizon_ray_entity_word(vertex, i)),
        bitcast<f32>(horizon_ray_entity_word(vertex, i + 1u)));
}
fn horizon_ray_entity_mix(t: horizon_RayTriangle, i: u32) -> vec2<f32> {
    return horizon_ray_entity_pair(t.corners.x, i) * t.weights.x
        + horizon_ray_entity_pair(t.corners.y, i) * t.weights.y
        + horizon_ray_entity_pair(t.corners.z, i) * t.weights.z;
}
fn horizon_ray_entity_color(t: horizon_RayTriangle) -> vec4<f32> {
    return unpack4x8unorm(horizon_ray_entity_word(t.corners.x, 6u)) * t.weights.x
        + unpack4x8unorm(horizon_ray_entity_word(t.corners.y, 6u)) * t.weights.y
        + unpack4x8unorm(horizon_ray_entity_word(t.corners.z, 6u)) * t.weights.z;
}
fn horizon_ray_entity_texel(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(horizon_ray_entity_atlas, horizon_ray_entity_sampler, uv, 0.0);
}
// The triangle of an entity layer hit: its geometry's first quad, plus two triangles a quad;
// on a retained instance, its custom data's.
fn horizon_ray_entity_triangle(c: RayIntersection) -> horizon_RayTriangle {
    if c.instance_index >= horizon_ray_space.retained_instances {
        var t = horizon_ray_triangle(c.instance_custom_data, c.primitive_index, c.barycentrics);
        t.corners += vec3<u32>(horizon_ray_retained_vertex);
        return t;
    }
    let first = horizon_ray_space.entity_bases[min(c.geometry_index, 2u)];
    return horizon_ray_triangle(first, c.primitive_index, c.barycentrics);
}
// Whether entity.wgsl draws the pixel of a quad whose layer word is `layer`, where its texel's
// alpha is `texel`, its vertex colour's `tint` and the dissolve mask's `mask`: only a quad of
// the layer (`RAY_ENTITY_QUAD`) that is lit, where `(texel * tint).a` is 0.1 or more. On the
// dissolve (overlay 38) a pixel whose tint is below the mask is gone, the rest tested with 1.
fn horizon_ray_entity_keeps(layer: u32, texel: f32, tint: f32, mask: f32) -> bool {
    let overlay = layer & 0xffffu;
    let unlit = overlay == 2u || overlay == 4u || overlay == 6u || overlay == 9u || overlay == 10u;
    if (layer & 0x10000000u) == 0u || unlit {
        return false;
    }
    if overlay == 38u {
        return tint >= mask && texel >= 0.1;
    }
    return texel * tint >= 0.1;
}
fn horizon_ray_entity_stops(c: RayIntersection) -> bool {
    let t = horizon_ray_entity_triangle(c);
    let layer = horizon_ray_entity_word(t.corners.x, 5u);
    var mask = 0.0;
    if (layer & 0xffffu) == 38u {
        mask = horizon_ray_entity_texel(horizon_ray_entity_mix(t, 9u)).a;
    }
    let texel = horizon_ray_entity_texel(horizon_ray_entity_mix(t, 3u)).a;
    return horizon_ray_entity_keeps(layer, texel, horizon_ray_entity_color(t).a, mask);
}
// `horizon_ray_tangent` from the uv steps `d1` and `d2` along the edges `e1` and `e2`.
fn horizon_ray_entity_tangent(d1: vec2<f32>, d2: vec2<f32>, e1: vec3<f32>, e2: vec3<f32>, normal: vec3<f32>) -> vec4<f32> {
    let s = select(-1.0, 1.0, d1.x * d2.y - d2.x * d1.y >= 0.0);
    let du = (e1 * d2.y - e2 * d1.y) * s;
    let dv = (e2 * d1.x - e1 * d2.x) * s;
    let along = du - normal * dot(normal, du);
    if length(along) < 1e-12 {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    let tangent = normalize(along);
    return vec4<f32>(tangent, select(-1.0, 1.0, dot(cross(normal, tangent), dv) >= 0.0));
}
// Vanilla's entity shade (entity.wgsl's `entity_light`), linear, on the side of the quad of `t`
// the ray along `direction` meets: from its first corner's normal in the stream, turned for the
// back of a quad drawn on both sides (`w` below 0); 1 for a quad vanilla doesn't shade (`w` 0:
// a block drawn as an entity, its shade in its colour).
fn horizon_ray_entity_shade(t: horizon_RayTriangle, direction: vec3<f32>) -> f32 {
    let n = unpack4x8snorm(horizon_ray_entity_word(t.corners.x, 8u));
    if abs(n.w) < 0.5 {
        return 1.0;
    }
    let normal = select(n.xyz, -n.xyz, n.w < 0.0 && dot(n.xyz, direction) > 0.0);
    let l0 = normalize(vec3<f32>(0.2, 1.0, -0.7));
    let l1 = normalize(vec3<f32>(-0.2, select(1.0, -1.0, horizon_ray_space.nether != 0u), 0.7));
    let shade = min(1.0, (max(0.0, dot(l0, normal)) + max(0.0, dot(l1, normal))) * 0.6 + 0.4);
    return horizon_decode(vec3<f32>(shade)).x;
}
// An entity layer hit as `trace_layers` gives it: the texel and the vertices' colour, light and
// uv, the face's normal against the ray, vanilla's shade for it; nothing of a block's.
fn horizon_ray_entity_hit(hit: RayIntersection, origin: vec3<f32>, direction: vec3<f32>) -> RayHit {
    let t = horizon_ray_entity_triangle(hit);
    let a = horizon_ray_entity_corner(t.corners.x);
    let e1 = horizon_ray_entity_corner(t.corners.y) - a;
    let e2 = horizon_ray_entity_corner(t.corners.z) - a;
    var normal = normalize(cross(e1, e2));
    let back = dot(normal, direction) > 0.0;
    if back {
        normal = -normal;
    }
    let uv = horizon_ray_entity_mix(t, 3u);
    let corner = horizon_ray_entity_pair(t.corners.x, 3u);
    let d1 = horizon_ray_entity_pair(t.corners.y, 3u) - corner;
    let d2 = horizon_ray_entity_pair(t.corners.z, 3u) - corner;
    // The light's 16 steps a level: sky in the low byte, block in the next.
    let words = vec3<u32>(horizon_ray_entity_word(t.corners.x, 7u),
        horizon_ray_entity_word(t.corners.y, 7u), horizon_ray_entity_word(t.corners.z, 7u));
    let sky = dot(vec3<f32>(words & vec3<u32>(0xffu)), t.weights);
    let block = dot(vec3<f32>((words >> vec3<u32>(8u)) & vec3<u32>(0xffu)), t.weights);
    let tint = horizon_decode_rgba(horizon_ray_entity_color(t));
    let shade = horizon_ray_entity_shade(t, direction);
    // `shaded_tint`: the product, as on a terrain hit (`ao` is 1 here).
    let horizon_shaded = vec4<f32>(tint.rgb * shade, tint.a);
    return RayHit(hit.t, origin + direction * hit.t, normal, uv,
        horizon_decode_rgba(horizon_ray_entity_texel(uv)), tint,
        shade, 1.0, min(vec2<f32>(block, sky) / 240.0, vec2<f32>(1.0)),
        0u, 0u, 0.0, 0u, false, back, true, horizon_ray_entity_tangent(d1, d2, e1, e2, normal),
        0u@shaded_tint@);
}
";

/// What the renderer gives the material prelude's LabPBR functions (`native_material.rs`); the
/// check and the entries below put a stand-in in its place.
macro_rules! pbr_stub {
    () => {
        "fn horizon_block_sample(uv: vec2<f32>, layer: u32, lod: f32) -> vec4<f32> {
    return vec4<f32>(1.0);
}
fn horizon_block_size() -> vec2<f32> {
    return vec2<f32>(16.0);
}
fn horizon_block_pbr(uv: vec2<f32>, layer: u32, lod: f32, map: u32) -> vec4<f32> {
    return select(vec4<f32>(0.5, 0.5, 1.0, 1.0), vec4<f32>(0.0), map == 1u);
}
fn horizon_block_pbr_present() -> bool {
    return false;
}
fn horizon_block_pbr_size() -> vec2<f32> {
    return vec2<f32>(16.0);
}
"
    };
}
pub const PBR_STUB: &str = pbr_stub!();

/// Entry points with the material stage's inputs: the renderer calls `material` from its own
/// terrain shader in the same way. The colours are decoded on the way in, and the result encoded
/// for the frame (`[frame] blend = "gamma"`; under `"linear"` the renderer stores it as it is).
const MATERIAL_ENTRIES: &str = concat!(
    pbr_stub!(),
    "\
struct HorizonMaterialIn {
    @builtin(position) position: vec4<f32>,
    @location(0) albedo: vec4<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) light: vec2<f32>,
    @location(3) lightmap: vec3<f32>,
    @location(4) world: vec3<f32>,
    @location(5) uv: vec2<f32>,
    @location(6) @interpolate(flat) translucent: u32,
    @location(7) @interpolate(flat) tags: u32,
    @location(8) @interpolate(flat) face: u32,
    @location(9) @interpolate(flat) fluid_quad: u32,
    @location(10) @interpolate(flat) emission: f32,
    @location(11) @interpolate(flat) material: u32,
    @location(12) shaded: vec4<f32>,
    @location(13) @interpolate(flat) shade: f32,
    @location(14) ao: f32,
}
@fragment fn horizon_fs_material(in: HorizonMaterialIn) -> @location(0) vec4<f32> {
    let fluid = select(0u, select(2u, 1u, in.translucent != 0u), in.fluid_quad != 0u);
    let normal = horizon_material_normal(in.world);
    let tangent = horizon_material_tangent(in.world, in.uv, normal);
    let horizon_shaded = horizon_decode_rgba(in.shaded);
    let input = MaterialInput(horizon_decode_rgba(in.albedo), horizon_decode_rgba(in.tint), in.shade,
        in.ao, in.light, horizon_decode(in.lightmap), in.world, in.uv, in.translucent != 0u, in.tags,
        in.material, in.position.xy / frame.resolution, in.face, fluid, in.emission, normal, tangent,
        vec3<f32>(0.0), 0.0@light_source@@lit_tint@@shaded_tint@);
    return horizon_encode_rgba(material(input).color);
}
"
);

/// Entry points with the sky stage's inputs: the renderer calls `sky` from
/// its own sky shader in the same way.
const SKY_ENTRIES: &str = "\
struct HorizonSkyIn {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
    @location(1) color: vec4<f32>,
}
@fragment fn horizon_fs_sky(in: HorizonSkyIn) -> @location(0) vec4<f32> {
    return horizon_encode_rgba(sky(SkyInput(normalize(in.direction), horizon_decode_rgba(in.color))));
}
";

/// An entry point with the vertex stage's inputs: the renderer calls `vertex` from its own
/// terrain shader in the same way. The tint is a colour, so it is decoded.
const VERTEX_ENTRIES: &str = "\
@vertex fn horizon_vs_vertex(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) light: vec2<f32>,
    @location(3) tint: vec4<f32>, @location(4) material: u32, @location(5) flags: u32,
    @location(6) shade: f32, @location(7) ao: f32, @location(8) shaded: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let horizon_shaded = horizon_decode_rgba(shaded);
    let input = VertexInput(position, uv, material, light, horizon_decode_rgba(tint), shade, ao,
        flags & 7u, flags >> 30u@shaded_tint@);
    return frame.projection * frame.view * vec4<f32>(vertex(input), 1.0);
}
";

/// `fn displacement(input: DisplacementInput) -> f32`, the material stage's function
/// under `[displacement]`.
pub const DISPLACEMENT_SIGNATURE: &str = "fn displacement(input: DisplacementInput) -> f32";

/// An entry point calling the material stage's `displacement`, after its entries under
/// `[displacement]`: the renderer calls it from its own terrain vertex shader in the same way.
const DISPLACEMENT_ENTRIES: &str = "@vertex fn horizon_vs_displacement(
    @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) material: u32, @location(4) height: f32,
) -> @builtin(position) vec4<f32> {
    let depth = displacement(DisplacementInput(position, normal, uv, material, height));
    return frame.projection * frame.view * vec4<f32>(position - normal * depth, 1.0);
}
";

/// An entry point with the weather stage's inputs: the renderer calls `weather` from its own
/// weather shader in the same way.
const WEATHER_ENTRIES: &str = "\
struct HorizonWeatherIn {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) light: vec2<f32>,
    @location(2) world: vec3<f32>,
    @location(3) @interpolate(flat) snow: u32,
}
@fragment fn horizon_fs_weather(in: HorizonWeatherIn) -> @location(0) vec4<f32> {
    let input = WeatherInput(horizon_decode_rgba(in.color), in.light, in.world, in.snow != 0u);
    return horizon_encode_rgba(weather(input));
}
";

/// The post stage's full-screen triangle, before its fragment entry.
macro_rules! post_vertex {
    () => {
        "\
struct HorizonPostOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn horizon_vs_post(@builtin(vertex_index) i: u32) -> HorizonPostOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: HorizonPostOut;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}
"
    };
}

/// The post stage's entry points, as the renderer builds them: a full-screen triangle, and a
/// fragment that calls the pack's `post` for each pixel. The chain's targets hold linear light,
/// so nothing converts, and the alpha of a pass that draws the next `previous` is ignored.
const POST_ENTRIES: &str = concat!(
    post_vertex!(),
    "\
@fragment fn horizon_fs_post(in: HorizonPostOut) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(previous, linear_sampler, in.uv, 0.0);
    return vec4<f32>(post(PostInput(in.uv, in.position.xy, color, depth_at(in.uv))).rgb, 1.0);
}
"
);

/// A post pass with an `output`, a target of the pack's own or a kept texture: the texture holds
/// data the pack reads back, so the pass's alpha stays.
const POST_ENTRIES_OUTPUT: &str = concat!(
    post_vertex!(),
    "\
@fragment fn horizon_fs_post(in: HorizonPostOut) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(previous, linear_sampler, in.uv, 0.0);
    return post(PostInput(in.uv, in.position.xy, color, depth_at(in.uv)));
}
"
);

/// The tonemap stage's entry points: a full-screen triangle at the window's size, and a fragment
/// that holds the result to what the display pass takes.
const TONEMAP_ENTRIES: &str = "\
struct HorizonTonemapOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn horizon_vs_tonemap(@builtin(vertex_index) i: u32) -> HorizonTonemapOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: HorizonTonemapOut;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}
@fragment fn horizon_fs_tonemap(in: HorizonTonemapOut) -> @location(0) vec4<f32> {
    let color = horizon_sanitize(textureSampleLevel(previous, linear_sampler, in.uv, 0.0).rgb);
    let out = tonemap(TonemapInput(in.uv, in.position.xy, color));
    return vec4<f32>(min(horizon_sanitize(out), vec3<f32>(frame.headroom)), 1.0);
}
";

/// An entry point with the entity stage's inputs, for the check: the renderer calls `entity`
/// from its own entity shader in the same way (`gfx/native_entity.rs`), decoding the colours on
/// the way in and encoding the result for a frame that stores gamma.
const ENTITY_ENTRIES: &str = "\
struct HorizonEntityIn {
    @builtin(position) position: vec4<f32>,
    @location(0) albedo: vec4<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) overlay: vec4<f32>,
    @location(3) lightmap: vec3<f32>,
    @location(4) shade: f32,
    @location(5) world: vec3<f32>,
    @location(6) light: vec2<f32>,
    @location(7) uv: vec2<f32>,
    @location(8) @interpolate(flat) full_light: u32,
    @location(9) @interpolate(flat) kind: u32,
    @location(10) @interpolate(flat) translucent: u32,
}
@fragment fn horizon_fs_entity(in: HorizonEntityIn) -> @location(0) vec4<f32> {
    let input = EntityInput(horizon_decode_rgba(in.albedo), horizon_decode_rgba(in.tint),
        horizon_decode_rgba(in.overlay), horizon_decode(in.lightmap), horizon_decode(vec3<f32>(in.shade)).x,
        in.world, in.full_light != 0u, horizon_entity_normal(in.world), in.kind, vec3<f32>(0.0),
        in.translucent != 0u, in.light, in.uv, in.position.xy / frame.resolution@light_source@);
    return horizon_encode_rgba(entity(input).color);
}
";

impl StageKind {
    pub const ALL: [StageKind; 7] = [
        StageKind::Sky,
        StageKind::Vertex,
        StageKind::Material,
        StageKind::Weather,
        StageKind::Post,
        StageKind::Tonemap,
        StageKind::Entity,
    ];

    /// The manifest's name for the stage (`[sky]`, `[vertex]`, `[material]`, `[weather]`,
    /// `[[post]]`, `[tonemap]`, `[entity]`).
    pub fn name(self) -> &'static str {
        match self {
            StageKind::Sky => "sky",
            StageKind::Material => "material",
            StageKind::Vertex => "vertex",
            StageKind::Weather => "weather",
            StageKind::Post => "post",
            StageKind::Tonemap => "tonemap",
            StageKind::Entity => "entity",
            StageKind::Compute => "compute",
            StageKind::BeforeWorld => "before_world",
        }
    }

    /// The function a pack defines: it has the stage's name, `compute` for both compute stages.
    pub fn function(self) -> &'static str {
        match self {
            StageKind::BeforeWorld => "compute",
            _ => self.name(),
        }
    }

    pub fn input(self) -> &'static Struct {
        match self {
            StageKind::Sky => &SKY_INPUT,
            StageKind::Material => &MATERIAL_INPUT,
            StageKind::Vertex => &VERTEX_INPUT,
            StageKind::Weather => &WEATHER_INPUT,
            StageKind::Post => &POST_INPUT,
            StageKind::Tonemap => &TONEMAP_INPUT,
            StageKind::Entity => &ENTITY_INPUT,
            StageKind::Compute | StageKind::BeforeWorld => &COMPUTE_INPUT,
        }
    }

    /// The type the function returns (empty: none).
    pub fn output(self) -> &'static str {
        match self {
            StageKind::Compute | StageKind::BeforeWorld => "",
            StageKind::Material | StageKind::Entity => SURFACE.name,
            StageKind::Vertex | StageKind::Tonemap => "vec3<f32>",
            StageKind::Sky | StageKind::Weather | StageKind::Post => "vec4<f32>",
        }
    }

    /// `fn <name>(input: <Input>) -> <Output>`, without `->` for a function returning nothing.
    pub fn signature(self) -> String {
        let output = match self.output() {
            "" => String::new(),
            ty => format!(" -> {ty}"),
        };
        format!(
            "fn {}(input: {}){output}",
            self.function(),
            self.input().name
        )
    }

    pub fn resources(self) -> &'static [Resource] {
        match self {
            StageKind::Post | StageKind::Compute => &POST_RESOURCES,
            StageKind::BeforeWorld => &BEFORE_WORLD_RESOURCES,
            StageKind::Sky | StageKind::Vertex | StageKind::Weather => std::slice::from_ref(&FRAME),
            StageKind::Material => &MATERIAL_RESOURCES,
            StageKind::Tonemap => &TONEMAP_RESOURCES,
            StageKind::Entity => &ENTITY_RESOURCES,
        }
    }

    /// Everything the pack's files may use, declared before them, for a pack without
    /// `[ray_tracing]`.
    pub fn prelude(self) -> String {
        self.prelude_traced(Tracing::None)
    }

    /// `prelude` with what `tracing` holds of the scene: in every stage the frame's two fields
    /// for it, and in the post, material, entity, compute and before-world stages `RayHit`, `trace`,
    /// `trace_any`, their `_layers` forms with `RAY_SOLID` and `RAY_TRANSLUCENT`,
    /// `hit_normal_map` and `hit_specular_map`, and outside the material stage, which has its
    /// own, `block_pbr_maps`.
    pub fn prelude_traced(self, tracing: Tracing) -> String {
        let rays = matches!(
            self,
            StageKind::Post
                | StageKind::Material
                | StageKind::Entity
                | StageKind::Compute
                | StageKind::BeforeWorld
        );
        self.prelude_rays(
            tracing, rays, false, false, false, false, false, false, false, false, false, false,
            false, false,
        )
    }

    /// `prelude_traced`, with the ray names only where `rays`: a material, entity, compute or
    /// before-world stage that calls none of the trace functions is built as it was before
    /// them. `kept`: the frame's epochs, for a pack with `[[resource]]`. With `entities` (the
    /// pack's files name `RAY_ENTITY`) the trace functions have the entity layer too. `fog`:
    /// `vanilla_fog`, for a pack with `[frame] fog = "pack"`.
    /// `shaded`: `SHADED_TINT` last in the terrain's inputs and `RayHit`, for a pack whose files
    /// name it. `lit`: `LIT_TINT` in `MaterialInput`, for a pack whose files name it.
    /// `moon_phase`: the frame's `moon_phase`, for a pack whose files name it.
    /// `light_source`: the input's `inputs::light_source` fields, for a pack whose files name
    /// one of them (`names_light_source`). `full_lightmap`: the frame's `full_lightmap`, for a
    /// pack whose files name it. `jitter`: the frame's `jitter` and `previous_jitter`, for a pack
    /// whose files name one of them (`names_jitter`).
    /// `looks`: a terrain hit reads its quad's look (`RAY_LOOKS`), for a stage that
    /// reads one (`check::RAY_LOOK_FIELDS`). With `lights` (the pack's files name the light
    /// functions, `names_lights`) the light sources are there. With `step` (the pack has
    /// `[ray_tracing] step = true` and the stage's files name one of `STEP_NAMES`, `names_step`)
    /// so is `trace_step` with its names.
    #[allow(clippy::too_many_arguments)]
    fn prelude_rays(
        self,
        tracing: Tracing,
        rays: bool,
        kept: bool,
        entities: bool,
        fog: bool,
        shaded: bool,
        moon_phase: bool,
        light_source: bool,
        full_lightmap: bool,
        jitter: bool,
        lit: bool,
        looks: bool,
        lights: bool,
        step: bool,
    ) -> String {
        let rays = rays && tracing != Tracing::None;
        let mut out = match tracing {
            Tracing::Scene if rays => String::from(RAY_ENABLE),
            _ => String::new(),
        };
        out += &format!(
            "// Horizon shader API {} prelude ({})\n",
            crate::API,
            crate::reference::PATH
        );
        out += &Frame::declared_wgsl(
            tracing != Tracing::None,
            kept,
            moon_phase,
            full_lightmap,
            jitter,
        );
        if kept {
            out += &crate::kept::reasons_wgsl();
        }
        let terrain = self.terrain();
        out += &crate::inputs::declared(self.input(), light_source, lit, shaded && terrain);
        if matches!(self, StageKind::Material | StageKind::Entity) {
            out += &SURFACE.wgsl();
        }
        if self == StageKind::Material {
            // The vertex stage's file joins the material's module (`compose`).
            out += &crate::inputs::shaded(&VERTEX_INPUT, shaded);
            out += &DISPLACEMENT_INPUT.wgsl();
        }
        for r in self.resources() {
            out += &format!("@group({GROUP}) @binding({}) {};\n", r.binding, r.ty);
        }
        out += COLOR_HELPERS;
        if fog {
            out += FOG_HELPERS;
        }
        match self {
            StageKind::Post | StageKind::Compute => {
                out += POST_HELPERS;
                out += POST_SURFACE;
                out += SHADOW_HELPERS;
            }
            StageKind::Material => {
                out += MATERIAL_HELPERS;
                out += SHADOW_HELPERS;
            }
            StageKind::Vertex | StageKind::BeforeWorld => out += SHADOW_HELPERS,
            StageKind::Entity => {
                out += ENTITY_HELPERS;
                out += SHADOW_HELPERS;
            }
            StageKind::Sky | StageKind::Weather | StageKind::Tonemap => {}
        }
        if rays {
            out += &crate::inputs::shaded(&ray_hit(entities), shaded);
            let looks = looks && tracing == Tracing::Scene;
            let scene = match tracing {
                Tracing::Scene => looked(RAY_SCENE, looks),
                _ => RAY_STUB.to_owned(),
            };
            out += &shaded_entries(&layered(&scene, entities), shaded);
            if looks {
                out += RAY_LOOKS;
                out += &look_shade("horizon_ray_look_shade");
            }
            if tracing == Tracing::Scene && entities {
                out += &shaded_entries(RAY_ENTITY_SCENE, shaded);
            }
            out += &layered(RAY_LAYERS, entities);
            if lights {
                out += match tracing {
                    Tracing::Scene => RAY_LIGHTS_SCENE,
                    _ => RAY_LIGHTS_STUB,
                };
            }
            if step {
                out += RAY_STEP;
                let step = match tracing {
                    Tracing::Scene => looked(RAY_STEP_SCENE, looks),
                    _ => RAY_STEP_STUB.to_owned(),
                };
                out += &shaded_entries(&layered(&step, entities), shaded);
            }
            // The material stage's renderer, or the check's stand-in, reads the LabPBR maps.
            if self != StageKind::Material {
                out += match (tracing, self) {
                    (Tracing::Scene, StageKind::Entity) => PBR_MAPS_ENTITY,
                    (Tracing::Scene, _) => PBR_MAPS_POST,
                    _ => PBR_STUB,
                };
                out += POST_PBR;
            }
            out += &layered(RAY_PBR, entities);
        }
        out
    }

    /// Whether the stage's input is the terrain's, which may end with `SHADED_TINT`.
    fn terrain(self) -> bool {
        matches!(self, StageKind::Material | StageKind::Vertex)
    }

    /// The entry points put after the pack's files.
    pub fn entries(self) -> &'static str {
        match self {
            StageKind::Sky => SKY_ENTRIES,
            StageKind::Material => MATERIAL_ENTRIES,
            StageKind::Vertex => VERTEX_ENTRIES,
            StageKind::Weather => WEATHER_ENTRIES,
            StageKind::Post => POST_ENTRIES,
            StageKind::Tonemap => TONEMAP_ENTRIES,
            StageKind::Entity => ENTITY_ENTRIES,
            // Each compute pass has its own (`compute_entries`).
            StageKind::Compute | StageKind::BeforeWorld => "",
        }
    }
}

/// The first binding of a pack's `[[textures]]` in group 3; their samplers follow at
/// `TEXTURE_SAMPLER_BINDING`.
pub const TEXTURE_BINDING: u32 = 16;
pub const TEXTURE_SAMPLER_BINDING: u32 = 24;

/// The declarations of a pack's `[[textures]]`, part of every stage's prelude.
pub fn texture_declarations(textures: &[crate::manifest::PackTexture]) -> String {
    let mut out = String::new();
    for (i, t) in textures.iter().enumerate() {
        out += &format!(
            "@group({GROUP}) @binding({}) var {}: texture_2d<f32>;\n\
             @group({GROUP}) @binding({}) var {}_sampler: sampler;\n",
            TEXTURE_BINDING + i as u32,
            t.id,
            TEXTURE_SAMPLER_BINDING + i as u32,
            t.id
        );
    }
    out
}

/// A stage's WGSL, and where each part of it came from.
#[derive(Debug)]
pub struct Composed {
    pub text: String,
    /// (the file, or `<prelude>` / `<entries>`, its byte range in `text`).
    pub parts: Vec<(String, std::ops::Range<usize>)>,
}

impl Composed {
    /// The file, line and column (1-based) of a byte offset in `text`.
    pub fn locate(&self, offset: usize) -> (&str, usize, usize) {
        let (file, range) = self
            .parts
            .iter()
            .find(|(_, r)| r.contains(&offset))
            .or(self.parts.last())
            .expect("a composed stage has parts");
        let before = &self.text[range.start..offset.clamp(range.start, range.end)];
        let line = before.matches('\n').count() + 1;
        let column = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
        (file, line, column)
    }
}

/// The binding of a pack's first post target (`[[post]] output`) in group 3, the others after.
pub const POST_TARGET_BINDING: u32 = 32;

/// The declarations of a pack's post targets, part of the post stage's prelude.
pub fn post_target_declarations(manifest: &crate::manifest::Manifest) -> String {
    manifest
        .post_targets()
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            format!(
                "@group({GROUP}) @binding({}) var {name}: texture_2d<f32>;\n",
                POST_TARGET_BINDING + i as u32
            )
        })
        .collect()
}

/// The motion guide's binding in group 3, in the 0.2 post passes of a pack with `[motion]`.
pub const MOTION_BINDING: u32 = 14;

/// The opaque scene's reversed depth and linear color, available to opted-in post passes.
pub const POST_OPAQUE_DEPTH_BINDING: u32 = 8;
pub const POST_OPAQUE_COLOR_BINDING: u32 = 15;

/// The motion guide's declarations, at `MOTION_BINDING`. The raw texture holds the engine's
/// objects (`frame_gen/motion.wgsl`): z 1 a matched one with xy its delta, w 1 one with nothing
/// to match. At depth 0 the engine writes the sky's camera motion from its unposed view (no bob
/// or spin, as the sky is drawn): xy the delta, w 1 behind last frame's view. `motion_at` adds
/// the camera's motion for the rest.
const MOTION: &str = "\
// The engine's objects' motion this frame (read it through `motion_at`).
@group(3) @binding(14) var scene_motion: texture_2d<f32>;
const MOTION_CAMERA: u32 = 0u;
const MOTION_OBJECT: u32 = 1u;
const MOTION_NEW: u32 = 2u;
const MOTION_NO_HISTORY: u32 = 3u;
struct MotionData {
    // Where the surface was last frame less where it is: top-left UV of the world's picture.
    delta: vec2<f32>,
    // A MOTION_ constant: how far to trust `delta`.
    kind: u32,
}
// How the surface `scene_depth` holds at `texel` moved since last frame.
fn motion_at(texel: vec2<i32>) -> MotionData {
    if frame.delta_time == 0.0 {
        return MotionData(vec2<f32>(0.0), MOTION_NO_HISTORY);
    }
    let size = vec2<i32>(textureDimensions(scene_depth));
    let t = clamp(texel, vec2<i32>(0), size - 1);
    let objects = vec2<i32>(textureDimensions(scene_motion));
    let object = textureLoad(scene_motion, clamp(t, vec2<i32>(0), objects - 1), 0);
    if object.z > 0.5 {
        return MotionData(object.xy, MOTION_OBJECT);
    }
    let depth = textureLoad(scene_depth, t, 0);
    // The sky, drawn without the view bob: the engine wrote its motion, w 1 behind last frame's view.
    if depth == 0.0 {
        if object.w > 0.5 {
            return MotionData(vec2<f32>(0.0), MOTION_NO_HISTORY);
        }
        return MotionData(object.xy, MOTION_CAMERA);
    }
    if object.w > 0.5 {
        return MotionData(vec2<f32>(0.0), MOTION_NEW);
    }
    // Still geometry, moved by the camera alone.
    let uv = (vec2<f32>(t) + 0.5) / vec2<f32>(size);
    let view = frame.projection_inverse * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let point = (frame.view_inverse * vec4<f32>(view.xyz / view.w, 0.0)).xyz + frame.camera_motion;
    let clip = frame.previous_view_projection * vec4<f32>(point, 1.0);
    // Behind last frame's camera: nothing there to reproject from.
    if clip.w <= 0.0 {
        return MotionData(vec2<f32>(0.0), MOTION_NO_HISTORY);
    }
    let then = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    return MotionData(then - uv, MOTION_CAMERA);
}
";

/// The binding of a pack's first `[[resource]]` in group 3, a texture's last-frame version the
/// next; the others after, two each.
pub const KEPT_BINDING: u32 = 64;

/// How a pass binds a kept resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptAccess {
    /// This frame's version, read: a read-only storage buffer, or a `texture_2d`.
    Read,
    /// This frame's version, written: a read-write storage buffer, or a write-only storage
    /// texture.
    Write,
    /// A texture's last-frame version (`_previous`), read as a `texture_2d`.
    Previous,
}

/// One kept resource a pass binds in group 3: `resource` indexes the manifest's
/// `kept.resources`. A pass's kept `output` is its attachment, not a binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeptBinding {
    pub binding: u32,
    pub resource: usize,
    pub access: KeptAccess,
}

/// The `[[resource]]`s the pass `stage` lists, in binding order: resource i at
/// `KEPT_BINDING + 2i`, written where it `writes` it, else read where it `reads` it; a
/// texture's `_previous` at the next binding where it reads that.
pub fn kept_bindings(
    manifest: &crate::manifest::Manifest,
    stage: &crate::Stage,
) -> Vec<KeptBinding> {
    let mut out = Vec::new();
    for (resource, r) in manifest.kept.resources.iter().enumerate() {
        let at = |binding, access| KeptBinding {
            binding,
            resource,
            access,
        };
        let binding = KEPT_BINDING + 2 * resource as u32;
        if stage.writes.contains(&r.name) {
            out.push(at(binding, KeptAccess::Write));
        } else if stage.reads.contains(&r.name) {
            out.push(at(binding, KeptAccess::Read));
        }
        if r.is_texture() && stage.reads.contains(&format!("{}_previous", r.name)) {
            out.push(at(binding + 1, KeptAccess::Previous));
        }
    }
    out
}

/// The declarations of `kept_bindings`, part of the pass's prelude: a buffer as
/// `array<element>`, read-write where written (an `atomic<u32>` read as `u32` where only read);
/// this frame's texture a storage texture where written, else read as `texture_2d`; last
/// frame's (`_previous`) read only.
pub fn kept_declarations(manifest: &crate::manifest::Manifest, stage: &crate::Stage) -> String {
    use crate::kept::{Element, Shape};
    let mut out = String::new();
    for b in kept_bindings(manifest, stage) {
        let r = &manifest.kept.resources[b.resource];
        let name = &r.name;
        let texel = r.format().map_or("f32", |f| f.texel());
        let ty = match (r.shape, b.access) {
            (_, KeptAccess::Previous) => format!("var {name}_previous: texture_2d<{texel}>"),
            (Shape::Buffer { element, .. }, KeptAccess::Write) => {
                format!("var<storage, read_write> {name}: array<{}>", element.wgsl())
            }
            (Shape::Buffer { element, .. }, KeptAccess::Read) => {
                let element = match element {
                    Element::AtomicU32 => Element::U32,
                    e => e,
                };
                format!("var<storage, read> {name}: array<{}>", element.wgsl())
            }
            (Shape::Fixed { format, .. } | Shape::Scaled { format, .. }, KeptAccess::Write) => {
                format!("var {name}: texture_storage_2d<{}, write>", format.wgsl())
            }
            (_, KeptAccess::Read) => format!("var {name}: texture_2d<{texel}>"),
        };
        out += &format!("@group({GROUP}) @binding({}) {ty};\n", b.binding);
    }
    out
}

/// A compute pass's entry point: its workgroup size, every invocation calling `compute`, with
/// `in_bounds` false beyond the dispatch's size.
fn compute_entries(stage: &crate::Stage) -> String {
    let [x, y, z] = stage.workgroup.unwrap_or([1, 1, 1]);
    let size = match stage.dispatch {
        Some(crate::kept::Dispatch::Cells([a, b, c])) => format!("vec3<u32>({a}u, {b}u, {c}u)"),
        _ => "vec3<u32>(vec2<u32>(frame.resolution), 1u)".to_owned(),
    };
    format!(
        "@compute @workgroup_size({x}, {y}, {z})\n\
         fn horizon_cs_compute(@builtin(global_invocation_id) cell: vec3<u32>, \
         @builtin(local_invocation_index) local: u32) {{\n    \
         let size = {size};\n    \
         compute(ComputeInput(cell, local, size, all(cell < size)));\n}}\n"
    )
}

/// The WGSL of the stage `kind` whose own file is `file`: the prelude, the
/// pack's shared files in order, the file, then the entry points. A
/// pack with `[ray_tracing]` gets `trace` without the scene (`Tracing::Stub`), which every
/// device takes.
pub fn compose(pack: &Pack, kind: StageKind, file: &str) -> Composed {
    compose_traced(pack, kind, file, None, false)
}

/// `compose` for a pack with `[ray_tracing]` where the renderer binds the scene: its post
/// stage traces it, and its material, entity, compute and before-world stages where they call a trace
/// function (`TRACE_NAMES`).
/// Such a module needs a device with ray queries.
pub fn compose_scene(pack: &Pack, kind: StageKind, file: &str) -> Composed {
    compose_traced(pack, kind, file, None, true)
}

/// The manifest entry of the stage `kind` whose file is `file`: with `at`, the entry at that
/// index of its list (`[[post]]` for a post or compute pass), since two entries may share a file
/// and differ in the rest; else the first.
pub fn entry<'a>(
    m: &'a crate::Manifest,
    kind: StageKind,
    file: &str,
    at: Option<usize>,
) -> Option<&'a crate::Stage> {
    let (list, compute) = match kind {
        StageKind::Post => (&m.post, false),
        StageKind::Compute => (&m.post, true),
        StageKind::BeforeWorld => (&m.kept.before_world, false),
        StageKind::Sky => return m.sky.as_ref(),
        StageKind::Material => return m.material.as_ref(),
        StageKind::Vertex => return m.vertex.as_ref(),
        StageKind::Weather => return m.weather.as_ref(),
        StageKind::Tonemap => return m.tonemap.as_ref(),
        StageKind::Entity => return m.entity.as_ref(),
    };
    let fits = |s: &&crate::Stage| s.file == file && s.compute == compute;
    match at {
        Some(at) => list.get(at).filter(fits),
        None => list.iter().find(fits),
    }
}

/// The names in WGSL `text` outside its comments (block comments nest), each with its byte
/// offset, in order.
pub(crate) fn identifiers(text: &str) -> Vec<(usize, &str)> {
    let bytes = text.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut names = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if text[at..].starts_with("//") {
            at += text[at..].find('\n').unwrap_or(text.len() - at);
        } else if text[at..].starts_with("/*") {
            let mut depth = 0;
            while at < bytes.len() {
                let (opens, closes) = (text[at..].starts_with("/*"), text[at..].starts_with("*/"));
                depth += i32::from(opens) - i32::from(closes);
                // A character of any width: `at` stays on a boundary.
                at += match opens || closes {
                    true => 2,
                    false => text[at..].chars().next().map_or(1, char::len_utf8),
                };
                if depth == 0 {
                    break;
                }
            }
        } else if word(bytes[at]) {
            let end = at + bytes[at..].iter().take_while(|b| word(**b)).count();
            names.push((at, &text[at..end]));
            at = end;
        } else {
            // A character of any width: `at` stays on a boundary.
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    names
}

/// The functions that trace a ray.
pub(crate) const TRACE_NAMES: [&str; 4] =
    ["trace", "trace_any", "trace_layers", "trace_any_layers"];

/// Whether the files of `pack`, a pack with `[ray_tracing]`, name `RAY_ENTITY` anywhere outside
/// a comment, dead code included: only then do its traced stages have the entity layer, and does
/// the renderer build it.
pub fn names_ray_entity(pack: &Pack) -> bool {
    let named = |file: &str| {
        let text = pack.source(file).unwrap_or_default();
        identifiers(text)
            .iter()
            .any(|(_, name)| *name == "RAY_ENTITY")
    };
    pack.manifest.ray_tracing.is_some() && pack.manifest.files().into_iter().any(named)
}

/// The names the light sources bring (`RAY_LIGHTS_SCENE`), taken only under
/// `[ray_tracing] lights = true`.
pub(crate) const LIGHT_NAMES: [&str; 5] = [
    "RayLight",
    "ray_light_count",
    "ray_light",
    "ray_lights_near",
    "ray_light_at",
];

/// The names `[ray_tracing] step = true` brings (`RAY_STEP`, `RAY_STEP_SCENE`): a stage whose
/// files name one of them gets them all, and with them needs the pack's `RayState` and `ray_step`.
pub(crate) const STEP_NAMES: [&str; 11] = [
    "trace_step",
    "RayCandidate",
    "candidate_stops",
    "candidate_albedo",
    "candidate_tint",
    "candidate_material",
    "candidate_tags",
    "candidate_fluid",
    "RAY_PASS",
    "RAY_STOP",
    "RAY_END",
];

/// Whether `pack` has `[ray_tracing] step = true`.
pub(crate) fn step_on(pack: &Pack) -> bool {
    (pack.manifest.ray_tracing.as_ref()).is_some_and(|r| r.step)
}

/// Whether `texts`, a stage's files, name one of `STEP_NAMES`.
fn names_step<'a>(mut texts: impl Iterator<Item = &'a str>) -> bool {
    texts.any(|text| {
        identifiers(text)
            .iter()
            .any(|(_, n)| STEP_NAMES.contains(n))
    })
}

/// Whether `pack` has `[ray_tracing] lights = true`.
pub(crate) fn lights_on(pack: &Pack) -> bool {
    (pack.manifest.ray_tracing.as_ref()).is_some_and(|r| r.lights)
}

/// Whether `texts`, a stage's files, name one of the light functions or `RayLight`.
fn names_lights<'a>(mut texts: impl Iterator<Item = &'a str>) -> bool {
    texts.any(|text| {
        identifiers(text)
            .iter()
            .any(|(_, n)| LIGHT_NAMES.contains(n))
    })
}

/// Whether the files of `pack` name `shaded_tint` anywhere outside a comment, dead code
/// included: only then do `MaterialInput`, `VertexInput` and `RayHit` have it (a false positive
/// only declares the field).
pub fn names_shaded_tint(pack: &Pack) -> bool {
    let named = |file: &str| {
        let text = pack.source(file).unwrap_or_default();
        identifiers(text)
            .iter()
            .any(|(_, name)| *name == crate::inputs::SHADED_TINT.name)
    };
    pack.manifest.files().into_iter().any(named)
}

/// Where an entry point's constructor of a terrain input, or the prelude's of `RayHit`, takes
/// `shaded_tint` (`horizon_shaded`) in a pack that names it.
pub const SHADED_TINT_ARGUMENT: &str = "@shaded_tint@";

/// `entries` with `SHADED_TINT_ARGUMENT` filled in for a pack that names `shaded_tint` or not.
pub fn shaded_entries(entries: &str, shaded: bool) -> String {
    entries.replace(
        SHADED_TINT_ARGUMENT,
        if shaded { ", horizon_shaded" } else { "" },
    )
}

/// Whether the files of `pack` name `lit_tint` anywhere outside a comment, dead code included:
/// only then does `MaterialInput` have it, and the renderer's terrain the varying that carries
/// it (a false positive only declares the field and takes the varying).
pub fn names_lit_tint(pack: &Pack) -> bool {
    files_name(pack, &[crate::inputs::LIT_TINT.name])
}

/// Where an entry point's constructor of `MaterialInput` takes `lit_tint` in a pack that names
/// it, after `LIGHT_SOURCE_ARGUMENT` and before `SHADED_TINT_ARGUMENT`.
pub const LIT_TINT_ARGUMENT: &str = "@lit_tint@";

/// `entries` with `LIT_TINT_ARGUMENT` filled in with `argument` (after a comma) for a pack that
/// names `lit_tint`, removed for one that doesn't (`None`).
pub fn lit_entries(entries: &str, argument: Option<&str>) -> String {
    entries.replace(LIT_TINT_ARGUMENT, argument.unwrap_or_default())
}

/// The check's stand-in for `lit_tint`: the product smoothed apart (the renderer smooths it at
/// each corner, in a varying the check's entry point has no room left for).
const LIT_TINT_STUB: &str = ", horizon_decode(in.shaded.rgb * in.lightmap)";

/// Whether the files of `pack` name `moon_phase` anywhere outside a comment, dead code included:
/// only then does its `Frame` have the field, so a pack that doesn't is built from the WGSL it
/// had before it.
pub fn names_moon_phase(pack: &Pack) -> bool {
    let named = |file: &str| {
        let text = pack.source(file).unwrap_or_default();
        identifiers(text)
            .iter()
            .any(|(_, name)| *name == "moon_phase")
    };
    pack.manifest.files().into_iter().any(named)
}

/// The names of `inputs::MATERIAL_LIGHT_SOURCE`.
const LIGHT_SOURCE_NAMES: [&str; 3] = [
    "light_source_tier",
    "light_source_whole",
    "light_source_threshold",
];

/// Whether the files of `pack` name one of `names` anywhere outside a comment, dead code included.
fn files_name(pack: &Pack, names: &[&str]) -> bool {
    let named = |file: &str| {
        let text = pack.source(file).unwrap_or_default();
        identifiers(text)
            .iter()
            .any(|(_, name)| names.contains(name))
    };
    pack.manifest.files().into_iter().any(named)
}

/// Whether the files of `pack` name `light_source_tier`, `light_source_whole` or
/// `light_source_threshold`: only then do `MaterialInput` and `EntityInput` have their
/// `inputs::light_source` fields, so a pack that doesn't is built from the WGSL it had before
/// them (a false positive only declares the fields).
pub fn names_light_source(pack: &Pack) -> bool {
    files_name(pack, &LIGHT_SOURCE_NAMES)
}

/// Whether the files of `pack` name `full_lightmap`: only then does its `Frame` have the field.
pub fn names_full_lightmap(pack: &Pack) -> bool {
    files_name(pack, &["full_lightmap"])
}

/// Whether the files of `pack` name `jitter` or `previous_jitter`: only then does its `Frame`
/// have both fields (`Frame::JITTER_FIELDS`).
pub fn names_jitter(pack: &Pack) -> bool {
    files_name(pack, &["jitter", "previous_jitter"])
}

/// Where an entry point's constructor of `MaterialInput` or `EntityInput` takes its
/// `inputs::light_source` fields in a pack that names them, before `SHADED_TINT_ARGUMENT`.
pub const LIGHT_SOURCE_ARGUMENT: &str = "@light_source@";

/// `entries` with `LIGHT_SOURCE_ARGUMENT` filled in with `arguments` (each after a comma) for a
/// pack that names the fields, removed for one that doesn't (`None`).
pub fn light_source_entries(entries: &str, arguments: Option<&str>) -> String {
    entries.replace(LIGHT_SOURCE_ARGUMENT, arguments.unwrap_or_default())
}

/// The check's stand-ins for the light-source fields, by stage (`light_source_entries`).
fn light_source_stub(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Material => ", 0u, false, 0.0",
        _ => ", 0u",
    }
}

/// Whether the stage's files, `texts`, name a trace function, or with `lights` (the pack's
/// `[ray_tracing] lights = true`) a light one, or with `step` (`step = true`) one of the step's.
/// Outside post passes, only stages that name one are built against the scene.
fn traces<'a>(mut texts: impl Iterator<Item = &'a str>, lights: bool, step: bool) -> bool {
    let ray = |(_, name): &(usize, &str)| {
        TRACE_NAMES.contains(name)
            || lights && LIGHT_NAMES.contains(name)
            || step && STEP_NAMES.contains(name)
    };
    texts.any(|text| identifiers(text).iter().any(ray))
}

/// `compose` (`scene`: `compose_scene`) of the entry `at` of the stage's list (`entry`).
pub(crate) fn compose_traced(
    pack: &Pack,
    kind: StageKind,
    file: &str,
    at: Option<usize>,
    scene: bool,
) -> Composed {
    let tracing = match (&pack.manifest.ray_tracing, scene) {
        (None, _) => Tracing::None,
        (Some(_), false) => Tracing::Stub,
        (Some(_), true) => Tracing::Scene,
    };
    let mut composed = Composed {
        text: String::new(),
        parts: Vec::new(),
    };
    let mut push = |name: &str, text: &str| {
        let start = composed.text.len();
        composed.text += text;
        if !text.ends_with('\n') {
            composed.text.push('\n');
        }
        composed
            .parts
            .push((name.to_owned(), start..composed.text.len()));
    };
    // The shared files, then the stage's own extra files, its file, and after it in the
    // material's module the vertex stage's.
    let m = &pack.manifest;
    let mut files: Vec<&str> = m
        .shared
        .iter()
        .map(String::as_str)
        .filter(|s| *s != file)
        .collect();
    let stage = entry(m, kind, file, at);
    let extras = stage.into_iter().flat_map(|s| &s.files);
    files.extend(
        extras
            .filter(|e| *e != file && !m.shared.contains(e))
            .map(String::as_str),
    );
    files.push(file);
    // They draw terrain together: the vertex stage's function is in the material's module.
    if kind == StageKind::Material
        && let Some(vertex) = &m.vertex
    {
        for extra in vertex.files.iter().chain(std::iter::once(&vertex.file)) {
            let own = stage.is_some_and(|s| s.files.contains(extra));
            if !own && !m.shared.contains(extra) {
                files.push(extra.as_str());
            }
        }
    }
    let source = |f: &&str| pack.source(f).unwrap_or_default();
    let rays = match kind {
        StageKind::Post => true,
        StageKind::Material | StageKind::Entity | StageKind::Compute | StageKind::BeforeWorld => {
            tracing != Tracing::None
                && traces(files.iter().map(source), lights_on(pack), step_on(pack))
        }
        _ => false,
    };
    let kept = !m.kept.resources.is_empty();
    let entities = rays && names_ray_entity(pack);
    let fog = m.fog == crate::Fog::Pack;
    let shaded = names_shaded_tint(pack);
    let moon_phase = names_moon_phase(pack);
    let light = names_light_source(pack);
    let full_lightmap = names_full_lightmap(pack);
    let jitter = names_jitter(pack);
    let lit = names_lit_tint(pack);
    // Only the stages whose own files name them: a pass that doesn't has no binding 80.
    let lights = rays && lights_on(pack) && names_lights(files.iter().map(source));
    let step = rays && step_on(pack) && names_step(files.iter().map(source));
    // Read off the stage as the stub builds it, which has the same files and `RayHit`.
    let looks = rays
        && tracing == Tracing::Scene
        && crate::check::reads_hit_looks(&compose_traced(pack, kind, file, at, false).text, step);
    let mut prelude = kind.prelude_rays(
        tracing,
        rays,
        kept,
        entities,
        fog,
        shaded,
        moon_phase,
        light,
        full_lightmap,
        jitter,
        lit,
        looks,
        lights,
        step,
    ) + &texture_declarations(&pack.manifest.textures);
    if matches!(kind, StageKind::Post | StageKind::Compute) {
        prelude += &post_target_declarations(&pack.manifest);
        if m.post_opaque {
            prelude += &format!(
                "@group({GROUP}) @binding({POST_OPAQUE_DEPTH_BINDING}) var opaque_depth: texture_depth_2d;\n\
                 @group({GROUP}) @binding({POST_OPAQUE_COLOR_BINDING}) var opaque_color: texture_2d<f32>;\n"
            );
        }
    }
    if kind == StageKind::Post && pack.manifest.motion.is_some() {
        prelude += MOTION;
    }
    if let Some(stage) = stage {
        prelude += &kept_declarations(m, stage);
    }
    push("<prelude>", &prelude);
    for f in &files {
        push(f, source(f));
    }
    let entries = |text: &str| {
        let stub = light.then(|| light_source_stub(kind));
        let lit = lit_entries(&shaded_entries(text, shaded), lit.then_some(LIT_TINT_STUB));
        light_source_entries(&lit, stub)
    };
    if kind == StageKind::Material && m.displacement.is_some() {
        push(
            "<entries>",
            &(entries(kind.entries()) + DISPLACEMENT_ENTRIES),
        );
    } else if let (StageKind::Compute | StageKind::BeforeWorld, Some(stage)) = (kind, stage) {
        push("<entries>", &compute_entries(stage));
    } else if kind == StageKind::Post && stage.is_some_and(|s| s.output.is_some()) {
        push("<entries>", POST_ENTRIES_OUTPUT);
    } else {
        push("<entries>", &entries(kind.entries()));
    }
    composed
}

/// Whether a material stage's `source` (with its prelude) calls `set_surface` itself, and so
/// needs the surface target, which a pack without it doesn't pay for.
pub fn calls_set_surface(source: &str) -> bool {
    let call = "set_surface(";
    source.matches(call).count() > MATERIAL_HELPERS.matches(call).count()
}
