// TFR motion field: reference depth -> forward motion (target_uv - source_uv), validity, edge.
// Computed in reference-frame space on a half-resolution (or configured) grid.
// Output: rgba16float = (motion.x, motion.y, validity, edge_metric).
// Conventions: Horizon reversed-Z (near=1, far/sky~0), WebGPU clip depth [0,1],
// UV origin top-left. `geo_reproject` = cur_vp*T(ref_cam-cur_cam)*ref_vp^-1,
// `sky_reproject` = cur_vp*ref_vp^-1 for rotation-only camera-relative VPs.

struct Params {
    geo_reproject: mat4x4<f32>,
    sky_reproject: mat4x4<f32>,
    // x,y: scene size; z,w: output size.
    sizes: vec4<f32>,
    // x: w epsilon, y: bounds epsilon, z: far/sky depth threshold, w: reserved.
    thresholds: vec4<f32>,
};

@group(0) @binding(0) var depth_tex: texture_2d<f32>;
@group(0) @binding(1) var output_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> params: Params;

fn uv_to_ndc(uv: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
}

fn ndc_to_uv(ndc: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

fn in_bounds(v: f32, eps: f32) -> bool {
    return v >= -eps && v <= 1.0 + eps;
}

fn load_depth(p: vec2<i32>, scene_size: vec2<i32>) -> f32 {
    let q = clamp(p, vec2<i32>(0), scene_size - 1);
    return textureLoad(depth_tex, q, 0).r;
}

fn edge_metric(uv: vec2<f32>, scene_size: vec2<f32>, center: f32) -> f32 {
    let texel = 1.0 / scene_size;
    let p = vec2<i32>(uv * scene_size);
    let d0 = load_depth(p + vec2<i32>(1, 0), vec2<i32>(scene_size));
    let d1 = load_depth(p + vec2<i32>(-1, 0), vec2<i32>(scene_size));
    let d2 = load_depth(p + vec2<i32>(0, 1), vec2<i32>(scene_size));
    let d3 = load_depth(p + vec2<i32>(0, -1), vec2<i32>(scene_size));
    let m = max(max(abs(center - d0), abs(center - d1)), max(abs(center - d2), abs(center - d3)));
    return clamp(m * 32.0, 0.0, 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let out_size = vec2<u32>(u32(params.sizes.z), u32(params.sizes.w));
    if (gid.x >= out_size.x || gid.y >= out_size.y) {
        return;
    }
    let scene_size = vec2<f32>(params.sizes.x, params.sizes.y);
    let uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) / vec2<f32>(out_size);
    let p = vec2<i32>(uv * scene_size);
    let depth = load_depth(p, vec2<i32>(scene_size));

    var matrix = params.geo_reproject;
    if (depth <= params.thresholds.z) {
        matrix = params.sky_reproject;
    }

    let ndc_xy = uv_to_ndc(uv);
    let clip = matrix * vec4<f32>(ndc_xy.x, ndc_xy.y, depth, 1.0);
    var out = vec4<f32>(0.0, 0.0, 0.0, edge_metric(uv, scene_size, depth));

    let w_eps = params.thresholds.x;
    let b_eps = params.thresholds.y;
    if (clip.w > w_eps) {
        let ndc = clip.xyz / clip.w;
        let target_uv = ndc_to_uv(ndc.xy);
        let target_depth = ndc.z;
        // Keep the forward vector even when its target leaves the viewport: the compose pass
        // walks motion from a destination sample towards its source, so a trailing-edge pixel
        // needs the vector stored at that pixel to find a source still inside the reference.
        // Only a vector the rgba16float target cannot hold (finite binary16 tops out at 65504;
        // a larger one would be stored as infinity), a non-finite projection or an invalid depth
        // makes the sample unusable. NaN fails the comparison and is rejected too.
        let motion = target_uv - uv;
        let valid = all(abs(motion) < vec2<f32>(65504.0))
            && in_bounds(target_depth, b_eps);
        if (valid) {
            out = vec4<f32>(motion, 1.0, out.w);
        }
    }
    textureStore(output_tex, vec2<i32>(i32(gid.x), i32(gid.y)), out);
}
