// Entities, dropped items and particles: camera-relative vertices textured
// from the shared 2D atlas, lit through the same lightmap as terrain.
struct Globals {
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    fog: vec4<f32>,
    sky_color: vec4<f32>,
    misc: vec4<f32>,
    inverse_view: mat4x4<f32>,
    sunrise: vec4<f32>,
    sun: vec4<f32>,
    // x: sky fog end, y: cloud fog end
    fog_ends: vec4<f32>,
    // Atlas rects (u0, v0, width, height) of end_sky.png and end_portal.png.
    end_portal_sprites: array<vec4<f32>, 2>,
    // x: vanilla's GameTime ((game time % 24000 + partial tick) / 24000)
    // y: Nether cardinal lighting, z: how many dynamic_lights are set.
    time: vec4<f32>,
    crumbling_sprite: vec4<f32>,
    // Selection outline parameters (read by terrain.wgsl).
    outline_params: vec4<f32>,
    // Horizon Extras' dynamic lights: camera-relative position and level (dynamic_light.rs).
    dynamic_lights: array<vec4<f32>, 16>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var lightmap: texture_2d<f32>;
@group(0) @binding(4) var lightmap_sampler: sampler;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) spherical: f32,
    @location(3) cylindrical: f32,
    // entity.vsh's overlayColor: the OverlayTexture texel (see overlay_texel).
    @location(5) overlay_color: vec4<f32>,
    @location(4) @interpolate(flat) overlay: u32,
    // Overlay 6: the sprite's v origin and height, for v repeated inside it.
    @location(6) @interpolate(flat) wrap: vec2<f32>,
    // Overlays 9 and 10: projection_from_position(gl_Position), the End portal's texProj0.
    @location(7) proj: vec4<f32>,
    @location(8) back_color: vec4<f32>,
    // entity.vsh's lightMapColor, multiplied in after the overlay.
    @location(9) light_color: vec4<f32>,
}
// Shipped light.glsl; normals are already transformed and RGBA8_SNORM packed.
fn entity_light(normal: vec3<f32>, nether: bool) -> f32 {
    let l0 = normalize(vec3(0.2, 1.0, -0.7));
    let l1 = normalize(vec3(-0.2, select(1.0, -1.0, nether), 0.7));
    return min(1.0, (max(0.0, dot(l0, normal)) + max(0.0, dot(l1, normal))) * 0.6 + 0.4);
}
// The OverlayTexture texel entity.vsh fetches (constants from overlay.rs):
// the hurt red for overlays 1 and 8, the white row at column 15 for the
// primed TNT flash (5) and at column overlay - 16 for 16..=31 (a creeper's
// swelling). Alpha 1 changes nothing.
fn overlay_texel(overlay: u32) -> vec4<f32> {
    if overlay == 1u || overlay == 8u { return HURT_OVERLAY; }
    if overlay == 5u { return vec4(1.0, 1.0, 1.0, WHITE_OVERLAY_ALPHA[15]); }
    if overlay >= 16u && overlay < 32u { return vec4(1.0, 1.0, 1.0, WHITE_OVERLAY_ALPHA[overlay - 16u]); }
    return vec4(1.0);
}
// Horizon Extras' dynamic lights (dynamic_light.rs): the block light, in the vertex's units
// (16 per level), a vertex at camera-relative `pos` gets from the frame's light sources, one
// level less per block. time.z of them are set: none when the extra is off, so no loop runs.
fn dynamic_block_light(pos: vec3<f32>) -> f32 {
    let count = min(u32(globals.time.z), 16u);
    var level = 0.0;
    for (var i = 0u; i < count; i++) {
        let l = globals.dynamic_lights[i];
        level = max(level, l.w - distance(pos, l.xyz));
    }
    return level * 16.0;
}
fn entity_vertex(position: vec3<f32>, uv: vec2<f32>,
    overlay: u32, color: vec4<f32>, light: vec4<u32>, normal: vec4<f32>) -> Out {
    var out: Out;
    out.clip = globals.view_proj * vec4(position, 1.0);
    out.proj = vec4(out.clip.xy * 0.5 + out.clip.w * 0.5, out.clip.zw);
    out.uv = uv;
    let l = vec2(max(f32(light.y), dynamic_block_light(position)), f32(light.x));
    let lm = textureSampleLevel(lightmap, lightmap_sampler,
        clamp(l / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0)), 0.0);
    // Celestial bodies, clouds, beacon beams, End portals and breaking cracks are unlit.
    let unlit = overlay == 2u || overlay == 4u || overlay == 6u || overlay == 9u || overlay == 10u
        || overlay == 11u || overlay == 12u;
    out.light_color = select(lm, vec4(1.0), unlit);
    // Overlay 6 (beacon beams): the light bytes carry the sprite's v range; 11 and 12
    // (cracks): the destroy stage sprite's origin.
    out.wrap = vec2(f32(light.x | (light.y << 8u)), f32(light.z | (light.w << 8u))) / 65535.0;
    // minecraft_mix_light: the vertex colour times the cardinal light only.
    var c = color;
    var back = c;
    if normal.w != 0.0 {
        c = vec4(c.rgb * entity_light(normal.xyz, globals.time.y != 0.0), c.a);
        back = select(c, vec4(back.rgb * entity_light(-normal.xyz, globals.time.y != 0.0), back.a), normal.w < 0.0);
    }
    out.color = c;
    out.back_color = back;
    out.overlay_color = overlay_texel(overlay);
    // Sky geometry (clouds, celestial bodies) is exempt from render-distance fog.
    out.spherical = select(length(position), 0.0, overlay >= 2u && overlay <= 4u);
    out.cylindrical = select(max(length(position.xz), abs(position.y)), 0.0, overlay >= 2u && overlay <= 4u);
    out.overlay = overlay;
    if overlay == 4u { out.spherical = length(position); }
    return out;
}
@vertex fn vs_entity(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) overlay: u32, @location(3) color: vec4<f32>, @location(4) light: vec4<u32>, @location(5) normal: vec4<f32>) -> Out {
    return entity_vertex(position, uv, overlay, color, light, normal);
}
struct CloudInfo {
    offset: vec4<f32>,
    colors: array<vec4<f32>, 4>,
}
@group(1) @binding(0) var<uniform> cloud: CloudInfo;
@vertex fn vs_cloud(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) overlay: u32, @location(3) color: vec4<f32>, @location(4) light: vec4<u32>, @location(5) normal: vec4<f32>) -> Out {
    return entity_vertex(position + cloud.offset.xyz, uv, overlay, cloud.colors[light.x], light, normal);
}
// rendertype_end_portal.fsh: the layer colours.
const PORTAL_COLORS = array<vec3<f32>, 16>(
    vec3(0.022087, 0.098399, 0.110818),
    vec3(0.011892, 0.095924, 0.089485),
    vec3(0.027636, 0.101689, 0.100326),
    vec3(0.046564, 0.109883, 0.114838),
    vec3(0.064901, 0.117696, 0.097189),
    vec3(0.063761, 0.086895, 0.123646),
    vec3(0.084817, 0.111994, 0.166380),
    vec3(0.097489, 0.154120, 0.091064),
    vec3(0.106152, 0.131144, 0.195191),
    vec3(0.097721, 0.110188, 0.187229),
    vec3(0.133516, 0.138278, 0.148582),
    vec3(0.070006, 0.243332, 0.235792),
    vec3(0.196766, 0.142899, 0.214696),
    vec3(0.047281, 0.315338, 0.321970),
    vec3(0.204675, 0.390010, 0.302066),
    vec3(0.080955, 0.314821, 0.661491),
);
// textureProj on a repeating texture that lives in the atlas at `rect`.
fn portal_sample(rect: vec4<f32>, p: vec4<f32>) -> vec3<f32> {
    let st = fract(p.xy / p.w);
    return textureSampleLevel(atlas, atlas_sampler, rect.xy + st * rect.zw, 0.0).rgb;
}
// texProj0 * end_portal_layer(layer): the row vector through mat4(scale * rotate),
// translate and SCALE_TRANSLATE in turn.
fn portal_layer(p: vec4<f32>, layer: f32) -> vec4<f32> {
    let a = radians((layer * layer * 4321.0 + layer * 9.0) * 2.0);
    let c = cos(a);
    let s = sin(a);
    let k = (4.5 - layer / 4.0) * 2.0;
    var v = vec4(k * (c * p.x - s * p.y), k * (s * p.x + c * p.y), p.z, p.w);
    v = vec4(v.x + 17.0 / layer * v.w, v.y + (2.0 + layer / 1.5) * (globals.time.x * 1.5) * v.w, v.z, v.w);
    return vec4(0.5 * v.x + 0.25 * v.w, 0.5 * v.y + 0.25 * v.w, v.z, v.w);
}
fn linear_fog(d: f32, start: f32, end: f32) -> f32 {
    if d <= start { return 0.0; }
    if d >= end { return 1.0; }
    return (d - start) / (end - start);
}
@fragment fn fs_entity(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if in.overlay == 6u { uv.y = in.wrap.x + fract(in.uv.y) * in.wrap.y; }
    if in.overlay == 11u || in.overlay == 12u {
        // Each crack's stage sprite origin comes in its light bytes (breaking::stage_light).
        uv = in.wrap + fract(in.uv) * globals.crumbling_sprite.zw;
    }
    var color = textureSample(atlas, atlas_sampler, uv) * select(in.back_color, in.color, front);
    if in.overlay == 9u || in.overlay == 10u {
        // rendertype_end_portal: end_sky and PORTAL_LAYERS (15 for portals, 16
        // for gateways) layers of end_portal.png projected from the screen.
        let layers = select(15, 16, in.overlay == 10u);
        var c = portal_sample(globals.end_portal_sprites[0], in.proj) * PORTAL_COLORS[0];
        for (var i = 0; i < layers; i++) {
            c += portal_sample(globals.end_portal_sprites[1], portal_layer(in.proj, f32(i + 1))) * PORTAL_COLORS[i];
        }
        color = vec4(c, 1.0);
    } else if in.overlay == 4u {
        // clouds.fsh: alpha fades out towards the cloud fog end.
        return vec4(color.rgb, color.a * (1.0 - linear_fog(in.spherical, 0.0, globals.fog_ends.y)));
    }
    if color.a < 0.1 { discard; }
    // entity.fsh calculateFinalColor: the overlay (hurt red, TNT white) over the
    // lit texture, then the lightmap, then the glint and fog.
    color = vec4(mix(in.overlay_color.rgb, color.rgb, in.overlay_color.a), color.a);
    color *= in.light_color;
    if in.overlay == 7u || in.overlay == 8u {
        // Enchantment glint (overlay 7, 8 with the hurt tint): the purple
        // enchanted_glint sheen drifting diagonally over the item's shape.
        let px = in.uv * vec2<f32>(textureDimensions(atlas));
        let t = globals.misc.y * 0.05;
        let band = sin((px.x + px.y) * 0.9 - t * 6.0) * 0.5 + 0.5;
        let band2 = sin((px.x - px.y) * 0.6 + t * 4.0) * 0.5 + 0.5;
        let shimmer = pow(band * 0.6 + band2 * 0.4, 2.0);
        let glint = vec3(0.50, 0.25, 0.80) * (0.25 + 0.75 * shimmer);
        color = vec4(min(color.rgb + glint * 0.6, vec3(1.0)), color.a);
    }
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color.rgb, globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}

@fragment fn fs_celestial(in: Out) -> @location(0) vec4<f32> {
    let color = textureSample(atlas, atlas_sampler, in.uv) * in.color * in.light_color;
    return color;
}

// RenderPipelines.WATER_MASK: depth only (the pipeline masks the colour off).
@fragment fn fs_water_mask() -> @location(0) vec4<f32> {
    return vec4(0.0);
}
