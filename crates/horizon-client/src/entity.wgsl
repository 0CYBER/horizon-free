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
}
@vertex fn vs_entity(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) overlay: u32, @location(3) color: vec4<f32>, @location(4) light: vec4<u32>) -> Out {
    var out: Out;
    out.clip = globals.view_proj * vec4(position, 1.0);
    out.uv = uv;
    let l = vec2(f32(light.y), f32(light.x));
    let lm = textureSampleLevel(lightmap, lightmap_sampler,
        clamp(l / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0)), 0.0);
    var c = color * lm;
    // Hurt overlay (OverlayTexture red tint).
    if overlay == 1u { c = vec4(mix(c.rgb, vec3(1.0, 0.0, 0.0), 0.3), c.a); }
    out.color = c;
    out.spherical = length(position);
    out.cylindrical = max(length(position.xz), abs(position.y));
    return out;
}
fn linear_fog(d: f32, start: f32, end: f32) -> f32 {
    if d <= start { return 0.0; }
    if d >= end { return 1.0; }
    return (d - start) / (end - start);
}
@fragment fn fs_entity(in: Out) -> @location(0) vec4<f32> {
    let color = textureSample(atlas, atlas_sampler, in.uv) * in.color;
    if color.a < 0.1 { discard; }
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color.rgb, globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}
