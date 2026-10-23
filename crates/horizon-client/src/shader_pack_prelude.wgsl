// Horizon shader pack prelude, api 2 (docs/shader-packs.md): placed before
// every stage's source. Bindings, the Frame and FrameExt uniforms, the
// full-screen vertex shader, G-buffer/shadow accessors and PBR/post helpers
// that hide the renderer's reversed depth.
struct Frame {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    inverse_view: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    resolution: vec4<f32>,
    time: vec4<f32>,
    sun: vec4<f32>,
    moon: vec4<f32>,
    sky_color: vec4<f32>,
    fog_color: vec4<f32>,
    fog: vec4<f32>,
    weather: vec4<f32>,
    camera_state: vec4<f32>,
    view_params: vec4<f32>,
    dimension: vec4<f32>,
}

struct FrameExt {
    previous_view: mat4x4<f32>,
    previous_projection: mat4x4<f32>,
    shadow_view: mat4x4<f32>,
    shadow_projection: mat4x4<f32>,
    shadow_inverse_view: mat4x4<f32>,
    shadow_inverse_projection: mat4x4<f32>,
    // xyz: previous camera world position, w: frame delta time (seconds)
    previous_camera: vec4<f32>,
    // x: shadow map resolution, y: shadow distance (blocks), z: sun_path_rotation (rad), w: shadow strength
    shadow_params: vec4<f32>,
    // x: eye block light (0..240), y: eye sky light (0..240), z: smoothed wetness (0..1), w: center linear depth
    environment: vec4<f32>,
}

@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var previous: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var nearest_sampler: sampler;
@group(0) @binding(5) var<uniform> frame: Frame;
@group(0) @binding(6) var<uniform> frame_ext: FrameExt;
@group(0) @binding(7) var scene_normal: texture_2d<f32>;
@group(0) @binding(8) var scene_material: texture_2d<f32>;
@group(0) @binding(9) var opaque_depth: texture_depth_2d;
@group(0) @binding(10) var shadow_depth: texture_depth_2d;
@group(0) @binding(11) var shadow_sampler: sampler_comparison;
@group(0) @binding(12) var noise_texture: texture_2d<f32>;
@group(0) @binding(13) var repeat_sampler: sampler;

struct StageIn {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn horizon_stage_vertex(@builtin(vertex_index) i: u32) -> StageIn {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: StageIn;
    out.position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2(p.x, 1.0 - p.y);
    return out;
}

// The world's depth at `uv`: 1 at the near plane, 0 for the sky.
fn depth_at(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(scene_depth, nearest_sampler, uv, 0);
}
// The opaque world's depth at `uv` (before translucent water and stained glass).
fn opaque_depth_at(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(opaque_depth, nearest_sampler, uv, 0);
}
fn is_sky(depth: f32) -> bool {
    return depth <= 0.0;
}
// Blocks from the eye along the view axis.
fn linear_depth(depth: f32) -> f32 {
    let near = frame.view_params.x;
    let far = frame.view_params.y;
    return near * far / ((far - near) * depth + near);
}
fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let v = frame.inverse_projection * vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    return v.xyz / v.w;
}
// World position relative to the eye (add frame.camera_position.xyz for the
// absolute one).
fn world_offset(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    return (frame.inverse_view * vec4(view_position(uv, depth), 1.0)).xyz;
}
// World-space surface normal at `uv` from the G-buffer (or reconstructed from depth).
fn world_normal_at(uv: vec2<f32>) -> vec3<f32> {
    let encoded = textureSampleLevel(scene_normal, nearest_sampler, uv, 0.0);
    if encoded.w > 0.001 {
        return normalize(encoded.xyz * 2.0 - 1.0);
    }
    let step_uv = frame.resolution.zw;
    let d0 = depth_at(uv);
    let p0 = world_offset(uv, d0);
    let px = world_offset(uv + vec2(step_uv.x, 0.0), depth_at(uv + vec2(step_uv.x, 0.0))) - p0;
    let py = world_offset(uv + vec2(0.0, step_uv.y), depth_at(uv + vec2(0.0, step_uv.y))) - p0;
    let n = cross(py, px);
    let len = length(n);
    if len > 1e-6 {
        return n / len;
    }
    return vec3(0.0, 1.0, 0.0);
}
// G-buffer material data at `uv`: r = block light (0..1), g = sky light (0..1),
// b = water/translucent mask (0..1), a = roughness (0..1).
fn material_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(scene_material, nearest_sampler, uv, 0.0);
}
// Reproject a camera-relative world position into previous-frame UV and clip depth.
fn reproject_uv(world_rel: vec3<f32>) -> vec3<f32> {
    let camera_delta = frame.camera_position.xyz - frame_ext.previous_camera.xyz;
    let prev_rel = world_rel + camera_delta;
    let clip = frame_ext.previous_projection * (frame_ext.previous_view * vec4(prev_rel, 1.0));
    if clip.w <= 1e-5 {
        return vec3(-1.0);
    }
    let ndc = clip.xyz / clip.w;
    return vec3(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
}
// Sample the directional sun/moon shadow map with 5-tap PCF and normal offset bias.
// Returns 1.0 in full light and 0.0 in full shadow.
fn sample_shadow(world_rel: vec3<f32>, world_normal: vec3<f32>) -> f32 {
    if frame_ext.shadow_params.w <= 0.001 {
        return 1.0;
    }
    let dist = length(world_rel);
    let max_dist = max(frame_ext.shadow_params.y, 16.0);
    if dist >= max_dist {
        return 1.0;
    }
    let light_dir = normalize(select(frame.moon.xyz, frame.sun.xyz, frame.sun.y >= -0.05));
    let ndotl = max(dot(world_normal, light_dir), 0.0);
    let bias_pos = world_rel + world_normal * (0.06 + 0.12 * (1.0 - ndotl));
    let light_clip = frame_ext.shadow_projection * (frame_ext.shadow_view * vec4(bias_pos, 1.0));
    let ndc = light_clip.xyz / max(light_clip.w, 1e-5);
    let shadow_uv = vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if any(shadow_uv < vec2(0.002)) || any(shadow_uv > vec2(0.998)) || ndc.z <= 0.0 || ndc.z >= 1.0 {
        return 1.0;
    }
    let texel = 1.0 / max(frame_ext.shadow_params.x, 512.0);
    let ref_z = clamp(ndc.z + 0.0008, 0.0, 1.0);
    var sum = textureSampleCompareLevel(shadow_depth, shadow_sampler, shadow_uv, ref_z);
    sum += textureSampleCompareLevel(shadow_depth, shadow_sampler, shadow_uv + vec2(texel, 0.0), ref_z);
    sum += textureSampleCompareLevel(shadow_depth, shadow_sampler, shadow_uv - vec2(texel, 0.0), ref_z);
    sum += textureSampleCompareLevel(shadow_depth, shadow_sampler, shadow_uv + vec2(0.0, texel), ref_z);
    sum += textureSampleCompareLevel(shadow_depth, shadow_sampler, shadow_uv - vec2(0.0, texel), ref_z);
    let fade = smoothstep(max_dist, max_dist * 0.82, dist);
    return mix(1.0, sum * 0.2, fade * frame_ext.shadow_params.w);
}
fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}
// ACES fitted filmic tonemapping curve (linear HDR -> linear 0..1).
fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3(0.0), vec3(1.0));
}
