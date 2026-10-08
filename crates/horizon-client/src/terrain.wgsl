// Terrain, sky and outline shaders, ported from the pinned vanilla
// terrain.vsh/fsh, fog.glsl and sample_lightmap.glsl.
struct Globals {
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    // environmental start/end, render-distance start/end
    fog: vec4<f32>,
    sky_color: vec4<f32>,
    // x: sky darkening, y: game time, z: aspect, w: tan(fov/2)
    misc: vec4<f32>,
    inverse_view: mat4x4<f32>,
    sunrise: vec4<f32>,
    // xyz: sun direction, w: star brightness
    sun: vec4<f32>,
    // x: sky fog end, y: cloud fog end, z: dark disc flag
    fog_ends: vec4<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var sprites: texture_2d_array<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;
@group(0) @binding(3) var lightmap: texture_2d<f32>;
@group(0) @binding(4) var lightmap_sampler: sampler;

struct TerrainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) color: vec4<f32>,
    @location(3) spherical: f32,
    @location(4) cylindrical: f32,
}
fn sample_lightmap(sky: f32, block: f32) -> vec4<f32> {
    let uv = vec2(block, sky) * 16.0;
    return textureSampleLevel(lightmap, lightmap_sampler,
        clamp(uv / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0)), 0.0);
}
@vertex fn vs_terrain(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec4<u32>,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    let pos = position + origin.xyz;
    var out: TerrainOut;
    out.clip = globals.view_proj * vec4(pos, 1.0);
    out.uv = uv;
    out.layer = layer;
    out.color = color * sample_lightmap(f32(light.x) / 16.0, f32(light.y) / 16.0);
    out.spherical = length(pos);
    out.cylindrical = max(length(pos.xz), abs(pos.y));
    return out;
}
fn linear_fog(d: f32, start: f32, end: f32) -> f32 {
    if d <= start { return 0.0; }
    if d >= end { return 1.0; }
    return (d - start) / (end - start);
}
fn apply_fog(color: vec4<f32>, spherical: f32, cylindrical: f32) -> vec4<f32> {
    let f = max(linear_fog(spherical, globals.fog.x, globals.fog.y),
                linear_fog(cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color.rgb, globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}
@fragment fn fs_solid(in: TerrainOut) -> @location(0) vec4<f32> {
    let color = textureSample(sprites, sprite_sampler, in.uv, i32(in.layer)) * in.color;
    if color.a < 0.5 { discard; }
    return apply_fog(vec4(color.rgb, 1.0), in.spherical, in.cylindrical);
}
@fragment fn fs_translucent(in: TerrainOut) -> @location(0) vec4<f32> {
    let color = textureSample(sprites, sprite_sampler, in.uv, i32(in.layer)) * in.color;
    if color.a < 0.01 { discard; }
    return apply_fog(color, in.spherical, in.cylindrical);
}

// Sky (SkyRenderer): the 10-vertex sky disc fan at y = 16 (radius 512) in
// the sky colour with sky fog (sky.fsh: fog 0..FogSkyEnd on the spherical
// distance, a step at FogSkyEnd on the cylindrical one), evaluated per pixel
// along the view ray with the fan's interpolated vertex distances; the
// clear colour (fog colour) elsewhere; the sunrise/sunset fan blended over
// it; the dark disc (y = -4) below the horizon when the camera is low.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}
@vertex fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1));
    var out: SkyOut;
    out.clip = vec4(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}
fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    return normalize((globals.inverse_view * vec4(ndc.x * globals.misc.z * globals.misc.w,
        ndc.y * globals.misc.w, -1.0, 0.0)).xyz);
}
const PI: f32 = 3.14159265;
// Interpolated (spherical, cylindrical) distance where the ray meets a disc
// fan at height y, or negative when it misses.
fn disc_distance(d: vec3<f32>, y: f32) -> vec2<f32> {
    if d.y * y <= 0.0 { return vec2(-1.0); }
    let p = d * (y / d.y);
    let h = length(p.xz);
    let phi = atan2(p.z, p.x);
    let sector = PI / 4.0;
    let delta = phi - (floor(phi / sector) * sector + sector * 0.5);
    let edge = 512.0 * cos(sector * 0.5) / cos(delta);
    if h > edge { return vec2(-1.0); }
    let s = h / edge;
    let ay = abs(y);
    let rim = sqrt(512.0 * 512.0 + y * y);
    return vec2(ay + s * (rim - ay), ay + s * (512.0 - ay));
}
fn sky_fog(color: vec3<f32>, dist: vec2<f32>) -> vec3<f32> {
    let end = globals.fog_ends.x;
    let f = max(linear_fog(dist.x, 0.0, end), linear_fog(dist.y, end, end));
    return mix(color, globals.fog_color.rgb, f * globals.fog_color.a);
}
fn rotate_x90(v: vec3<f32>) -> vec3<f32> { return vec3(v.x, -v.z, v.y); }
// Weight of the fan centre where the ray crosses the sunrise fan (0 when missed).
fn sunrise_weight(d: vec3<f32>) -> f32 {
    let alpha = globals.sunrise.a;
    if alpha <= 0.001 { return 0.0; }
    // sin(sunAngle) < 0 flips the fan to the other horizon; sun.z = -sin(sunAngle).
    let flip = select(0.0, 180.0, -globals.sun.z < 0.0);
    let theta = radians(flip + 90.0);
    let c = cos(theta);
    let s = sin(theta);
    var best = 0.0;
    let apex = rotate_x90(vec3(-100.0 * s, 100.0 * c, 0.0));
    for (var i = 0; i < 16; i++) {
        let a0 = f32(i) * 2.0 * PI / 16.0;
        let a1 = f32(i + 1) * 2.0 * PI / 16.0;
        let l0 = vec3(sin(a0) * 120.0, cos(a0) * 120.0, -cos(a0) * 40.0 * alpha);
        let l1 = vec3(sin(a1) * 120.0, cos(a1) * 120.0, -cos(a1) * 40.0 * alpha);
        let r0 = rotate_x90(vec3(l0.x * c - l0.y * s, l0.x * s + l0.y * c, l0.z));
        let r1 = rotate_x90(vec3(l1.x * c - l1.y * s, l1.x * s + l1.y * c, l1.z));
        // Moller-Trumbore from the origin along d.
        let e1 = r0 - apex;
        let e2 = r1 - apex;
        let pv = cross(d, e2);
        let det = dot(e1, pv);
        if abs(det) < 1e-6 { continue; }
        let tv = -apex;
        let u = dot(tv, pv) / det;
        let qv = cross(tv, e1);
        let v = dot(d, qv) / det;
        let t = dot(e2, qv) / det;
        if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 {
            best = max(best, 1.0 - u - v);
        }
    }
    return best;
}
@fragment fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let d = view_ray(in.ndc);
    var color = globals.fog_color.rgb;
    let dist = disc_distance(d, 16.0);
    if dist.x >= 0.0 {
        color = sky_fog(globals.sky_color.rgb, dist);
    }
    let w = sunrise_weight(d);
    if w > 0.0 {
        color = mix(color, globals.sunrise.rgb, w * globals.sunrise.a);
    }
    return vec4(color, 1.0);
}
@fragment fn fs_dark_disc(in: SkyOut) -> @location(0) vec4<f32> {
    let d = view_ray(in.ndc);
    let dist = disc_distance(d, -4.0);
    if dist.x < 0.0 { discard; }
    return vec4(sky_fog(vec3(0.0), dist), 1.0);
}

struct LineOut { @builtin(position) clip: vec4<f32> }
@vertex fn vs_line(@location(0) position: vec3<f32>) -> LineOut {
    var out: LineOut;
    out.clip = globals.view_proj * vec4(position, 1.0);
    return out;
}
@fragment fn fs_line() -> @location(0) vec4<f32> { return vec4(0.0, 0.0, 0.0, 0.4); }
