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

// Sky: vanilla draws a sky disc in the sky colour and blends towards the fog
// colour at the horizon; sunrise/sunset tints the horizon facing the sun.
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
@fragment fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let view = normalize((globals.inverse_view * vec4(in.ndc.x * globals.misc.z * globals.misc.w,
        in.ndc.y * globals.misc.w, -1.0, 0.0)).xyz);
    let up = clamp(view.y, -1.0, 1.0);
    // Fog band towards the horizon (vanilla sky end distance blending).
    let horizon = 1.0 - smoothstep(0.0, 0.35, up);
    var color = mix(globals.sky_color.rgb, globals.fog_color.rgb, horizon);
    let toward_sun = max(dot(normalize(vec3(view.x, 0.0, view.z) + vec3(0.0001)), normalize(vec3(globals.sun.x, 0.0, globals.sun.z) + vec3(0.0001))), 0.0);
    let band = (1.0 - smoothstep(-0.1, 0.4, abs(up))) * pow(toward_sun, 3.0);
    color = mix(color, globals.sunrise.rgb, band * globals.sunrise.a);
    if up < 0.0 {
        // Void below the horizon.
        color = mix(color, globals.sky_color.rgb * 0.2, smoothstep(0.0, -0.3, up) * (1.0 - globals.misc.x));
    }
    return vec4(color, 1.0);
}

struct LineOut { @builtin(position) clip: vec4<f32> }
@vertex fn vs_line(@location(0) position: vec3<f32>) -> LineOut {
    var out: LineOut;
    out.clip = globals.view_proj * vec4(position, 1.0);
    return out;
}
@fragment fn fs_line() -> @location(0) vec4<f32> { return vec4(0.0, 0.0, 0.0, 0.4); }
