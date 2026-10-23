// Horizon shader pack prelude, api 2 additions (docs/shader-packs.md): placed
// after the api 1 prelude before the stages of an `api = 2` pack, so api 1
// packs keep every name they may have declared themselves.
struct FrameExt {
    previous_view: mat4x4<f32>,
    previous_projection: mat4x4<f32>,
    shadow_view: mat4x4<f32>,
    shadow_projection: mat4x4<f32>,
    shadow_inverse_view: mat4x4<f32>,
    shadow_inverse_projection: mat4x4<f32>,
    // xyz: previous camera world position, w: frame delta time (seconds)
    previous_camera: vec4<f32>,
    // x: shadow map resolution, y: shadow distance (blocks), z: 0,
    // w: 1 while the shadow map is drawn this frame, else 0
    shadow_params: vec4<f32>,
    // x: eye block light (0..240), y: eye sky light (0..240), z: smoothed
    // rain (0..1), w: 0
    environment: vec4<f32>,
}

@group(0) @binding(6) var<uniform> frame_ext: FrameExt;
@group(0) @binding(7) var scene_normal: texture_2d<f32>;
@group(0) @binding(8) var scene_material: texture_2d<f32>;
@group(0) @binding(9) var opaque_depth: texture_depth_2d;
@group(0) @binding(10) var shadow_depth: texture_depth_2d;
@group(0) @binding(11) var shadow_sampler: sampler_comparison;
@group(0) @binding(12) var noise_texture: texture_2d<f32>;
@group(0) @binding(13) var repeat_sampler: sampler;

// The opaque world's depth at `uv` (before translucent water and stained glass).
fn opaque_depth_at(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(opaque_depth, nearest_sampler, uv, 0);
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

// ACES fitted filmic tonemapping curve (linear HDR -> linear 0..1).
fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3(0.0), vec3(1.0));
}
