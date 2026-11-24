// TFR synthetic composition from history using motion field.
// Motion field stores forward motion per reference sample: m = target_uv - source_uv.
// For a current output pixel uv, we search for reference/source s with s + m(s) ~= uv,
// so history is sampled near uv - m (iteratively refined by resampling motion at candidates).

struct Params {
    // x: reprojection strength [0,1], y: history stability [0,0.35],
    // z: reserved, w: use external edge texture (0/1).
    mode: vec4<f32>,
    // x,y: source/history size in texels.
    texel: vec4<f32>,
};

@group(0) @binding(0) var history_tex: texture_2d<f32>;
@group(0) @binding(1) var history_sampler: sampler;
@group(0) @binding(2) var motion_tex: texture_2d<f32>;
@group(0) @binding(3) var motion_sampler: sampler;
@group(0) @binding(4) var edge_tex: texture_2d<f32>;
@group(0) @binding(5) var edge_sampler: sampler;
@group(0) @binding(6) var output_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var<uniform> params: Params;

fn clamp_uv(uv: vec2<f32>) -> vec2<f32> {
    let e = 0.5 / max(params.texel.xy, vec2<f32>(1.0));
    return clamp(uv, e, vec2<f32>(1.0) - e);
}

fn sample_history(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(history_tex, history_sampler, clamp_uv(uv), 0.0);
}

fn nearest_history(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(history_tex));
    let p = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - 1);
    return textureLoad(history_tex, p, 0);
}

fn edge_at(uv: vec2<f32>) -> f32 {
    if (params.mode.w > 0.5) {
        return textureSampleLevel(edge_tex, edge_sampler, clamp_uv(uv), 0.0).r;
    }
    return 0.0;
}

// Motion is read through a linear sampler, so a footprint that mixes valid and invalid texels
// would blend a shortened vector (invalid texels store 0) with a validity above 0.5. Accept a
// filtered sample only when every tap in its footprint is valid (validity filters to 1.0).
fn all_valid(m: vec4<f32>) -> bool {
    return m.z >= 0.999;
}

fn candidate_valid(uv: vec2<f32>) -> bool {
    // A fill candidate is usable only if motion marks it valid and edge is acceptable.
    let m = textureSampleLevel(motion_tex, motion_sampler, clamp_uv(uv), 0.0);
    let e = max(m.w, edge_at(uv));
    return all_valid(m) && e <= 0.95;
}

fn deterministic_fill(dst_uv: vec2<f32>) -> vec4<f32> {
    // Deterministic outward search (fixed direction order/radii). Return first valid
    // history candidate; if none found, return neutral color (never invalid/edge history).
    let center = vec2<f32>(0.5, 0.5);
    let dir = normalize(center - dst_uv + vec2<f32>(1e-6, 0.0));
    let perp = vec2<f32>(-dir.y, dir.x);
    let dirs = array<vec2<f32>, 8>(
        dir, -dir, perp, -perp,
        normalize(dir + perp), normalize(dir - perp),
        normalize(-dir + perp), normalize(-dir - perp)
    );
    let step = 1.0 / max(params.texel.x, 1.0);
    for (var i: i32 = 1; i <= 16; i = i + 1) {
        let t = f32(i) * step;
        for (var d: u32 = 0u; d < 8u; d = d + 1u) {
            let uv = dst_uv + dirs[d] * t;
            if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
                if (candidate_valid(uv)) {
                    return sample_history(uv);
                }
            }
        }
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(output_tex));
    if (i32(gid.x) >= size.x || i32(gid.y) >= size.y) {
        return;
    }
    let uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) / vec2<f32>(size);
    let strength = clamp(params.mode.x, 0.0, 1.0);
    let stability = clamp(params.mode.y, 0.0, 0.35);

    // Deterministic backward search: s ~= uv - m(s)*strength.
    // Rule: sample motion at current src; if valid, advance src from it.
    // If invalid on first iteration, try dst-uv motion once as a hint; otherwise stop.
    // After search, require motion at final src to be valid and to land on uv (no cycling/guessing).
    var src = clamp_uv(uv);
    // Keep the unclamped candidate: clamping hides a pan-revealed strip and would stretch the
    // border texel instead of using the hole fill. Clamp only coordinates used for sampling.
    var src_raw = uv;
    var valid = false;
    for (var i: i32 = 0; i < 3; i = i + 1) {
        let m = textureSampleLevel(motion_tex, motion_sampler, src, 0.0);
        if (all_valid(m)) {
            src_raw = uv - m.xy * strength;
            src = clamp_uv(src_raw);
            valid = true;
        } else {
            if (i == 0) {
                let mc = textureSampleLevel(motion_tex, motion_sampler, uv, 0.0);
                if (all_valid(mc)) {
                    src_raw = uv - mc.xy * strength;
                    src = clamp_uv(src_raw);
                    valid = true;
                }
            }
            break;
        }
    }
    if (valid) {
        let mfinal = textureSampleLevel(motion_tex, motion_sampler, src, 0.0);
        // The final source must project back onto this pixel within one texel; a search that
        // did not converge (or bounced between two regions) takes the hole fill instead.
        let residual = src_raw + mfinal.xy * strength - uv;
        let tol = 1.0 / max(params.texel.xy, vec2<f32>(1.0));
        valid = all_valid(mfinal) && all(abs(residual) <= tol);
    }

    var color: vec4<f32>;
    if (!valid || any(src_raw < vec2<f32>(0.0)) || any(src_raw > vec2<f32>(1.0))) {
        color = deterministic_fill(uv);
    } else {
        let msrc = textureSampleLevel(motion_tex, motion_sampler, src, 0.0);
        let edge = max(msrc.w, edge_at(src));
        let base = select(sample_history(src), nearest_history(src), edge > 0.6);
        if (stability > 0.0 && edge < 0.95) {
            let texel = 1.0 / max(params.texel.xy, vec2<f32>(1.0));
            let n0 = sample_history(src + vec2<f32>( texel.x, 0.0));
            let n1 = sample_history(src + vec2<f32>(-texel.x, 0.0));
            let n2 = sample_history(src + vec2<f32>(0.0,  texel.y));
            let n3 = sample_history(src + vec2<f32>(0.0, -texel.y));
            let avg = (base + n0 + n1 + n2 + n3) * 0.2;
            color = vec4<f32>(mix(base.rgb, avg.rgb, stability * (1.0 - edge)), base.a);
        } else {
            color = base;
        }
    }

    color = vec4<f32>(clamp(color.rgb, vec3<f32>(0.0), vec3<f32>(65000.0)), clamp(color.a, 0.0, 1.0));
    textureStore(output_tex, vec2<i32>(i32(gid.x), i32(gid.y)), color);
}
