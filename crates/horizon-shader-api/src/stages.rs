//! The stages a pack can fill, each a WGSL function with a fixed signature
//! that Horizon calls inside its own passes, and the WGSL put around a
//! pack's files to build a stage: the prelude before (the inputs and the
//! resources), the entry points after.
use crate::inputs::{
    DISPLACEMENT_INPUT, ENTITY_INPUT, Frame, MATERIAL_INPUT, POST_INPUT, RAY_HIT, SKY_INPUT,
    SURFACE, Struct, TONEMAP_INPUT, VERTEX_INPUT, WEATHER_INPUT,
};
use crate::manifest::Api;
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
    /// 0.2: the picture for an output (the display), once per pixel of it after the post chain,
    /// before the camera overlays and the HUD.
    Tonemap,
    /// 0.2: the colour of surfaces that are no block terrain (entities, block entities, items,
    /// particles), before the alpha test, the glint and fog. Checked, not yet drawn.
    Entity,
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
        doc: "The post chain's picture at the window's size (without a post chain, the world's at the world's size), linear light: what `input.color` samples, for a curve that reads around the pixel.",
    },
    LINEAR_SAMPLER,
    NEAREST_SAMPLER,
];

/// 0.2's colour boundary, in every 0.2 prelude: vanilla's 2.2 power both ways.
const COLOR_HELPERS: &str = "\
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

const POST_RESOURCES: [Resource; 12] = [
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
    Resource {
        binding: 8,
        name: "history",
        ty: "var history: texture_2d<f32>",
        doc: "Last frame's input to the last pass (what the pass before it wrote), for temporal passes (TAA, denoising): put such a pass second to last and it reads its own previous output; reproject with `frame.previous_view_projection`. Black on the first frame, after a gap of over half a second, and with a single pass.",
    },
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

/// `block_light_color` of the material and entity preludes: the arguments are the sampler the
/// stage has for it and the line's return statement.
macro_rules! block_light_color {
    ($sampler:literal, $light:literal) => {
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
",
            $light,
            "}
"
        )
    };
}

/// The entity prelude's helpers: the coloured-light grid, and the normal its entries give.
const ENTITY_HELPERS: &str = concat!(
    block_light_color!(
        "block_light_sampler",
        "    return vec4<f32>(horizon_decode(c.rgb * 4.0), c.a);\n"
    ),
    "\
// The surface's normal from the derivatives of its position, turned toward the camera.
fn horizon_entity_normal(position: vec3<f32>) -> vec3<f32> {
    let n = normalize(cross(dpdx(position), dpdy(position)));
    return select(n, -n, dot(n, position) > 0.0);
}
"
);

/// What the material entries (the checker's below and the renderer's) share: the
/// surface's normal and tangent from the derivatives of its position and uv. The arguments are
/// the line's return statements of `block_light_color`, `block_texture` and
/// `block_texture_level`, the helpers that read colour, and the two sums of the tangent.
macro_rules! material_helpers {
    ($light:literal, $texture:literal, $texture_level:literal, $tangent:literal) => {
        concat!(
            block_light_color!("reflection_sampler", $light),
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
",
            $texture,
            "}
// `block_texture` at mip level `lod` (0 the full-size texture).
fn block_texture_level(uv: vec2<f32>, lod: f32) -> vec4<f32> {
",
            $texture_level,
            "}
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
",
            $tangent,
            "    if length(t) < 1e-12 {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    let tangent = normalize(t - normal * dot(normal, t));
    return vec4<f32>(tangent, select(-1.0, 1.0, dot(cross(normal, tangent), b) >= 0.0));
}
"
        )
    };
}

const MATERIAL_HELPERS: &str = material_helpers!(
    "    return vec4<f32>(c.rgb * 4.0, c.a);\n",
    "    return horizon_block_sample(uv, horizon_block_layer, horizon_block_lod);\n",
    "    return horizon_block_sample(uv, horizon_block_layer, lod);\n",
    // 0.1's WGSL is frozen: its tangent points where `uv.x` shrinks (#16082).
    "    let t = dp2perp * duv1.x + dp1perp * duv2.x;\n    \
     let b = dp2perp * duv1.y + dp1perp * duv2.y;\n"
);

/// 0.2's: the `[[lights]]` colours and the block texture decoded (the LabPBR maps are data).
const MATERIAL_HELPERS_0_2: &str = material_helpers!(
    "    return vec4<f32>(horizon_decode(c.rgb * 4.0), c.a);\n",
    "    return horizon_decode_rgba(horizon_block_sample(uv, horizon_block_layer, horizon_block_lod));\n",
    "    return horizon_decode_rgba(horizon_block_sample(uv, horizon_block_layer, lod));\n",
    // Both sums carry the sign of the screen's handedness against `normal`, negative for a
    // normal that faces the camera (the framebuffer's y points down).
    "    let side = select(-1.0, 1.0, dot(dp1, dp2perp) >= 0.0);\n    \
     let t = (dp2perp * duv1.x + dp1perp * duv2.x) * side;\n    \
     let b = (dp2perp * duv1.y + dp1perp * duv2.y) * side;\n"
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
/// `horizon_RaySpace`, 32 bytes: the camera in the scene's space (three floats), how many
/// layers a sprite array holds (a `u32`), then how many of the scene's instances, its first
/// ones, are solid terrain's (a `u32`).
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

/// The directive the scene's prelude starts with: WGSL takes it before any declaration.
const RAY_ENABLE: &str = "enable wgpu_ray_query;\n";

/// The scene's layers and the functions that read solid terrain alone, with the scene or without
/// it: `trace_layers` and `trace_any_layers` come before (`RAY_SCENE`, `RAY_STUB`).
const RAY_LAYERS: &str = "\
// The scene's layers, to OR together for `trace_layers` and `trace_any_layers`: solid terrain
// (cut-outs too), and what the translucent pass draws (water, stained glass, ice).
const RAY_SOLID: u32 = 1u;
const RAY_TRANSLUCENT: u32 = 2u;
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

/// `trace_layers` and `trace_any_layers` without the scene: every ray misses.
const RAY_STUB: &str = "\
// The nearest surface of `layers` from the camera-relative `origin` along the unit vector
// `direction`, within `max_distance` blocks. The scene isn't there (`frame.has_ray_tracing` is
// 0): a miss.
fn trace_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> RayHit {
    return RayHit(-1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec2<f32>(0.0), vec4<f32>(0.0),
        vec4<f32>(0.0), vec2<f32>(0.0), 0u, 0u, 0.0, 0u, false, vec4<f32>(0.0), 0u);
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

/// A traced post pass's, against the scene: its `nearest_sampler` (at full size, the one mip
/// level a hit reads, as the terrain's sampler there) and the scene's first sprite array.
const PBR_MAPS_POST: &str = pbr_maps!("nearest_sampler", "horizon_ray_sprites_0");

/// A traced post pass's `block_pbr_maps`, as the material prelude's.
const POST_PBR: &str = "\
// LabPBR: whether the resource packs ship normal (`_n`) or specular (`_s`) maps beside their
// block textures. Without them `hit_normal_map` and `hit_specular_map` give their defaults.
fn block_pbr_maps() -> bool {
    return horizon_block_pbr_present();
}
";

/// The LabPBR maps at a hit, in every stage that traces, with the scene or without it.
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
    if hit.distance < 0.0 {
        return vec4<f32>(0.5, 0.5, 1.0, 1.0);
    }
    return horizon_hit_pbr(hit, 0u);
}
// The LabPBR specular map at a hit, as `block_specular_map_level(uv, 0.0)` (r perceptual
// smoothness, g F0 or a metal's id, b porosity or subsurface scattering, a emission, 1 none).
// (0, 0, 0, 0) where the texture has none, and on a miss.
fn hit_specular_map(hit: RayHit) -> vec4<f32> {
    if hit.distance < 0.0 {
        return vec4<f32>(0.0);
    }
    return horizon_hit_pbr(hit, 1u);
}
";

/// `trace_layers` and `trace_any_layers` through the scene the renderer binds
/// (gfx/ray_scene.rs): a hit is a triangle of a quad in the store, read as terrain.wgsl reads
/// the mesher's vertices.
const RAY_SCENE: &str = "\
struct horizon_RaySpace {
    // The camera in the scene's space: from the corner of the scene's anchor column, in blocks.
    camera: vec3<f32>,
    // The layers one sprite array holds (terrain.wgsl's `SPRITE_SPLIT`).
    sprite_split: u32,
    // The scene's instances of solid terrain, its first ones: the others are translucent terrain.
    solid_instances: u32,
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
    rayQueryInitialize(&query, horizon_ray_scene, RayDesc(flags, layers & 3u, 0.0,
        max(max_distance, 0.0), origin + horizon_ray_space.camera, direction));
    while rayQueryProceed(&query) {
        if horizon_ray_stops(rayQueryGetCandidateIntersection(&query)) {
            rayQueryConfirmIntersection(&query);
        }
    }
    return rayQueryGetCommittedIntersection(&query);
}
// The nearest surface of `layers` (`RAY_SOLID`, `RAY_TRANSLUCENT` or both) from the
// camera-relative `origin` along the unit vector `direction`, within `max_distance` blocks;
// `distance` is negative where there is none.
fn trace_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> RayHit {
    let hit = horizon_ray_cast(origin, direction, max_distance, layers, false);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        return RayHit(-1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec2<f32>(0.0), vec4<f32>(0.0),
            vec4<f32>(0.0), vec2<f32>(0.0), 0u, 0u, 0.0, 0u, false, vec4<f32>(0.0), 0u);
    }
    let t = horizon_ray_triangle(hit.instance_custom_data, hit.primitive_index, hit.barycentrics);
    let a = horizon_ray_corner(t.corners.x);
    let e1 = horizon_ray_corner(t.corners.y) - a;
    let e2 = horizon_ray_corner(t.corners.z) - a;
    var normal = normalize(cross(e1, e2));
    if dot(normal, direction) > 0.0 {
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
    // The colour's alpha byte, as the terrain reads it (mesher.rs's `fluid_face` and
    // `emitting`): a fluid quad's high nibble is 8 + its face, every other quad's 0xF; the low
    // nibble is 15 minus the face's HDR tier, codes above 4 the same tiers shining all over.
    let flags = horizon_ray_word(t.corners.x, 5u) >> 24u;
    let high = flags >> 4u;
    let fluid_quad = high >= 8u && (high & 7u) < 6u;
    // Water is in the translucent layer and lava in the solid one, as their passes draw them.
    let translucent = hit.instance_index >= horizon_ray_space.solid_instances;
    let fluid = select(0u, select(2u, 1u, translucent), fluid_quad);
    let code = 15u - (flags & 15u);
    let tier = select(code, code - 4u, code > 4u);
    let emission = select(min(f32(tier) * 7.0 / 15.0, 1.0), 0.0, tier == 0u);
    return RayHit(hit.t, origin + direction * hit.t, normal, uv,
        horizon_decode_rgba(horizon_ray_texel(layer, uv)), vec4<f32>(horizon_decode(color.rgb), 1.0),
        min(vec2<f32>(block, sky) / 240.0, vec2<f32>(1.0)), material, tags, emission, fluid,
        translucent, horizon_ray_tangent(t, e1, e2, normal), layer);
}
// Whether anything of `layers` lies within `max_distance` blocks of `origin` along `direction`.
fn trace_any_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> bool {
    return horizon_ray_cast(origin, direction, max_distance, layers, true).kind
        != RAY_QUERY_INTERSECTION_NONE;
}
";

/// The post stage's entry points, as the renderer builds them: a full-screen
/// triangle, and a fragment that calls the pack's `post` for each pixel.
const POST_ENTRIES: &str = "\
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
@fragment fn horizon_fs_post(in: HorizonPostOut) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(previous, linear_sampler, in.uv, 0.0);
    return post(PostInput(in.uv, in.position.xy, color, depth_at(in.uv)));
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

/// Entry points with the material stage's inputs: the renderer calls
/// `material` from its own terrain shader in the same way. The arguments are the line's
/// `MaterialInput` and return statements.
macro_rules! material_entries {
    ($input:literal, $result:literal) => {
        concat!(
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
}
@fragment fn horizon_fs_material(in: HorizonMaterialIn) -> @location(0) vec4<f32> {
    let fluid = select(0u, select(2u, 1u, in.translucent != 0u), in.fluid_quad != 0u);
    let normal = horizon_material_normal(in.world);
    let tangent = horizon_material_tangent(in.world, in.uv, normal);
",
            $input,
            $result,
            "}
"
        )
    };
}

const MATERIAL_ENTRIES: &str = material_entries!(
    "    let input = MaterialInput(in.albedo, in.tint, in.light, in.lightmap, in.world, in.uv, in.translucent != 0u, in.tags, in.material,
        in.position.xy / frame.resolution, in.face, fluid, in.emission, normal, tangent, vec3<f32>(0.0), 0.0);
",
    "    return material(input).color;\n"
);

/// 0.2's: the colours decoded on the way in, and the result encoded for the frame
/// (`[frame] blend = "gamma"`; under `"linear"` the renderer stores it as it is).
const MATERIAL_ENTRIES_0_2: &str = material_entries!(
    "    let input = MaterialInput(horizon_decode_rgba(in.albedo), horizon_decode_rgba(in.tint), in.light,
        horizon_decode(in.lightmap), in.world, in.uv, in.translucent != 0u, in.tags, in.material,
        in.position.xy / frame.resolution, in.face, fluid, in.emission, normal, tangent, vec3<f32>(0.0), 0.0);
",
    "    return horizon_encode_rgba(material(input).color);\n"
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
    return sky(SkyInput(normalize(in.direction), in.color));
}
";

const SKY_ENTRIES_0_2: &str = "\
struct HorizonSkyIn {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
    @location(1) color: vec4<f32>,
}
@fragment fn horizon_fs_sky(in: HorizonSkyIn) -> @location(0) vec4<f32> {
    return horizon_encode_rgba(sky(SkyInput(normalize(in.direction), horizon_decode_rgba(in.color))));
}
";

/// An entry point with the weather stage's inputs: the renderer calls `weather` from its own
/// weather shader in the same way.
const VERTEX_ENTRIES: &str = "\
@vertex fn horizon_vs_vertex(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) light: vec2<f32>,
    @location(3) tint: vec4<f32>, @location(4) material: u32, @location(5) flags: u32,
) -> @builtin(position) vec4<f32> {
    let input = VertexInput(position, uv, material, light, tint, flags & 7u, flags >> 31u);
    return frame.projection * frame.view * vec4<f32>(vertex(input), 1.0);
}
";

/// 0.2's: the tint is a colour, so it is decoded too.
const VERTEX_ENTRIES_0_2: &str = "\
@vertex fn horizon_vs_vertex(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) light: vec2<f32>,
    @location(3) tint: vec4<f32>, @location(4) material: u32, @location(5) flags: u32,
) -> @builtin(position) vec4<f32> {
    let input = VertexInput(position, uv, material, light, horizon_decode_rgba(tint), flags & 7u, flags >> 31u);
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

const WEATHER_ENTRIES: &str = "\
struct HorizonWeatherIn {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) light: vec2<f32>,
    @location(2) world: vec3<f32>,
    @location(3) @interpolate(flat) snow: u32,
}
@fragment fn horizon_fs_weather(in: HorizonWeatherIn) -> @location(0) vec4<f32> {
    return weather(WeatherInput(in.color, in.light, in.world, in.snow != 0u));
}
";

const WEATHER_ENTRIES_0_2: &str = "\
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

/// The post stage's entry points under 0.2: the chain's targets hold linear light, so nothing
/// converts, and the pass's alpha is ignored.
const POST_ENTRIES_0_2: &str = "\
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
@fragment fn horizon_fs_post(in: HorizonPostOut) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(previous, linear_sampler, in.uv, 0.0);
    return vec4<f32>(post(PostInput(in.uv, in.position.xy, color, depth_at(in.uv))).rgb, 1.0);
}
";

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

/// An entry point with the entity stage's inputs, for the check: the renderer will call
/// `entity` from its own entity shader in the same way, decoding the colours on the way in and
/// encoding the result for a frame that stores gamma.
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
    @location(8) @interpolate(flat) emission: f32,
    @location(9) @interpolate(flat) kind: u32,
    @location(10) @interpolate(flat) translucent: u32,
}
@fragment fn horizon_fs_entity(in: HorizonEntityIn) -> @location(0) vec4<f32> {
    let input = EntityInput(horizon_decode_rgba(in.albedo), horizon_decode_rgba(in.tint),
        horizon_decode_rgba(in.overlay), horizon_decode(in.lightmap), horizon_decode(vec3<f32>(in.shade)).x,
        in.world, in.emission, horizon_entity_normal(in.world), in.kind, vec3<f32>(0.0),
        in.translucent != 0u, in.light, in.uv, in.position.xy / frame.resolution);
    return horizon_encode_rgba(entity(input).color);
}
";

impl Api {
    /// The stages the line has, in `StageKind::ALL`'s order.
    pub fn stages(self) -> &'static [StageKind] {
        match self {
            Api::V0_1 => &StageKind::ALL[..5],
            Api::V0_2 => &StageKind::ALL,
        }
    }
}

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
        }
    }

    /// The function a pack defines: it has the stage's name.
    pub fn function(self) -> &'static str {
        self.name()
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
        }
    }

    /// The type the function returns.
    pub fn output(self) -> &'static str {
        match self {
            StageKind::Material | StageKind::Entity => SURFACE.name,
            StageKind::Vertex | StageKind::Tonemap => "vec3<f32>",
            StageKind::Sky | StageKind::Weather | StageKind::Post => "vec4<f32>",
        }
    }

    /// `fn <name>(input: <Input>) -> <Output>`.
    pub fn signature(self) -> String {
        format!(
            "fn {}(input: {}) -> {}",
            self.function(),
            self.input().name,
            self.output()
        )
    }

    pub fn resources(self) -> &'static [Resource] {
        match self {
            StageKind::Post => &POST_RESOURCES,
            StageKind::Sky | StageKind::Vertex | StageKind::Weather => std::slice::from_ref(&FRAME),
            StageKind::Material => &MATERIAL_RESOURCES,
            StageKind::Tonemap => &TONEMAP_RESOURCES,
            StageKind::Entity => &ENTITY_RESOURCES,
        }
    }

    /// Everything the pack's files may use under the line `api`, declared before them, for a
    /// pack without `[ray_tracing]`.
    pub fn prelude(self, api: Api) -> String {
        self.prelude_traced(api, Tracing::None)
    }

    /// `prelude` with what `tracing` holds of the scene: in every stage the frame's two fields
    /// for it, and in the post and material stages `RayHit`, `trace`, `trace_any`, their
    /// `_layers` forms with `RAY_SOLID` and `RAY_TRANSLUCENT`, `hit_normal_map` and
    /// `hit_specular_map` (and in the post stage `block_pbr_maps`).
    pub fn prelude_traced(self, api: Api, tracing: Tracing) -> String {
        let rays = matches!(self, StageKind::Post | StageKind::Material);
        self.prelude_rays(api, tracing, rays)
    }

    /// `prelude_traced`, with the ray names only where `rays`: a material stage that calls
    /// none of the trace functions is built as it was before them.
    fn prelude_rays(self, api: Api, tracing: Tracing, rays: bool) -> String {
        let rays = rays && tracing != Tracing::None;
        let mut out = match tracing {
            Tracing::Scene if rays => String::from(RAY_ENABLE),
            _ => String::new(),
        };
        out += match api {
            Api::V0_1 => "// Horizon shader API prelude (docs/api/shaders-reference.md)\n",
            Api::V0_2 => "// Horizon shader API 0.2 prelude (docs/api/shaders-reference-0.2.md)\n",
        };
        out += &Frame::declared(api, tracing != Tracing::None).wgsl();
        out += &self.input().wgsl();
        if matches!(self, StageKind::Material | StageKind::Entity) {
            out += &SURFACE.wgsl();
        }
        if self == StageKind::Material {
            // The vertex stage's file joins the material's module (`compose`).
            out += &VERTEX_INPUT.wgsl();
            out += &DISPLACEMENT_INPUT.wgsl();
        }
        for r in self.resources() {
            out += &format!("@group({GROUP}) @binding({}) {};\n", r.binding, r.ty);
        }
        if api == Api::V0_2 {
            out += COLOR_HELPERS;
        }
        match self {
            StageKind::Post => {
                out += POST_HELPERS;
                out += POST_SURFACE;
                out += SHADOW_HELPERS;
            }
            StageKind::Material => {
                out += match api {
                    Api::V0_1 => MATERIAL_HELPERS,
                    Api::V0_2 => MATERIAL_HELPERS_0_2,
                };
                out += SHADOW_HELPERS;
            }
            StageKind::Vertex => out += SHADOW_HELPERS,
            StageKind::Entity => {
                out += ENTITY_HELPERS;
                out += SHADOW_HELPERS;
            }
            StageKind::Sky | StageKind::Weather | StageKind::Tonemap => {}
        }
        if rays {
            out += &RAY_HIT.wgsl();
            out += match tracing {
                Tracing::Scene => RAY_SCENE,
                _ => RAY_STUB,
            };
            out += RAY_LAYERS;
            // The material stage's renderer, or the check's stand-in, reads the LabPBR maps.
            if self == StageKind::Post {
                out += match tracing {
                    Tracing::Scene => PBR_MAPS_POST,
                    _ => PBR_STUB,
                };
                out += POST_PBR;
            }
            out += RAY_PBR;
        }
        out
    }

    /// The entry points put after the pack's files under the line `api`.
    pub fn entries(self, api: Api) -> &'static str {
        match (api, self) {
            (Api::V0_1, StageKind::Sky) => SKY_ENTRIES,
            (Api::V0_1, StageKind::Material) => MATERIAL_ENTRIES,
            (Api::V0_1, StageKind::Vertex) => VERTEX_ENTRIES,
            (Api::V0_1, StageKind::Weather) => WEATHER_ENTRIES,
            (Api::V0_1, StageKind::Post) => POST_ENTRIES,
            (Api::V0_2, StageKind::Sky) => SKY_ENTRIES_0_2,
            (Api::V0_2, StageKind::Material) => MATERIAL_ENTRIES_0_2,
            (Api::V0_2, StageKind::Vertex) => VERTEX_ENTRIES_0_2,
            (Api::V0_2, StageKind::Weather) => WEATHER_ENTRIES_0_2,
            (Api::V0_2, StageKind::Post) => POST_ENTRIES_0_2,
            // 0.1 has no tonemap or entity stage: a manifest with one is refused before it is built.
            (_, StageKind::Tonemap) => TONEMAP_ENTRIES,
            (_, StageKind::Entity) => ENTITY_ENTRIES,
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

/// The WGSL of the stage `kind` whose own file is `file`: the prelude, the
/// pack's shared files in order, the file, then the entry points, all of the pack's line. A
/// pack with `[ray_tracing]` gets `trace` without the scene (`Tracing::Stub`), which every
/// device takes.
pub fn compose(pack: &Pack, kind: StageKind, file: &str) -> Composed {
    compose_traced(pack, kind, file, false)
}

/// `compose` for a pack with `[ray_tracing]` where the renderer binds the scene: its post
/// stage traces it, and its material stage where that calls a trace function (`TRACE_NAMES`).
/// Such a module needs a device with ray queries.
pub fn compose_scene(pack: &Pack, kind: StageKind, file: &str) -> Composed {
    compose_traced(pack, kind, file, true)
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
const TRACE_NAMES: [&str; 4] = ["trace", "trace_any", "trace_layers", "trace_any_layers"];

/// Whether a material stage's files, `texts`, name a trace function: only then does its prelude
/// have them and the renderer build its terrain against the scene.
fn traces<'a>(mut texts: impl Iterator<Item = &'a str>) -> bool {
    let ray = |(_, name): &(usize, &str)| TRACE_NAMES.contains(name);
    texts.any(|text| identifiers(text).iter().any(ray))
}

fn compose_traced(pack: &Pack, kind: StageKind, file: &str, scene: bool) -> Composed {
    let api = pack.manifest.api;
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
    let stage = match kind {
        StageKind::Sky => m.sky.as_ref(),
        StageKind::Material => m.material.as_ref(),
        StageKind::Vertex => m.vertex.as_ref(),
        StageKind::Weather => m.weather.as_ref(),
        StageKind::Post => m.post.iter().find(|s| s.file == file),
        StageKind::Tonemap => m.tonemap.as_ref(),
        StageKind::Entity => m.entity.as_ref(),
    };
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
        StageKind::Material => tracing != Tracing::None && traces(files.iter().map(source)),
        _ => false,
    };
    let mut prelude =
        kind.prelude_rays(api, tracing, rays) + &texture_declarations(&pack.manifest.textures);
    if kind == StageKind::Post {
        prelude += &post_target_declarations(&pack.manifest);
        if api == Api::V0_2 && pack.manifest.motion.is_some() {
            prelude += MOTION;
        }
    }
    push("<prelude>", &prelude);
    for f in &files {
        push(f, source(f));
    }
    if kind == StageKind::Material && m.displacement.is_some() {
        push(
            "<entries>",
            &(kind.entries(api).to_owned() + DISPLACEMENT_ENTRIES),
        );
    } else {
        push("<entries>", kind.entries(api));
    }
    composed
}

/// Whether a material stage's `source` (with its prelude) calls `set_surface` itself, and so
/// needs the surface target, which a pack without it doesn't pay for.
pub fn calls_set_surface(source: &str) -> bool {
    let call = "set_surface(";
    source.matches(call).count() > MATERIAL_HELPERS.matches(call).count()
}
