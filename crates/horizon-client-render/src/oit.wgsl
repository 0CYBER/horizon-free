// Improved transparency (the options screen's order-independent transparency),
// appended to terrain.wgsl. Vanilla 26.3 draws its translucent layer three
// times: once to find each pixel's nearest and furthest see-through depth (the
// depth bounds), once to add each fragment's absorbance into a small wavelet
// transform over that range (the transmittance), and once to weight every
// fragment's colour by what the transform says lies in front of it (the
// accumulation). A last full-screen pass puts the sum over the opaque world.
//
// Eight coefficients in two RGBA16F targets: [0] is the mean absorbance, [1]
// the widest wavelet, [2..4] the next level, [4..8] the finest, one per depth
// bin pair.
struct OitParams {
    // x, y: the projection's [2][2] and [3][2], which turn a device depth into
    // a distance along the view axis: y / (depth + x).
    projection: vec4<f32>,
}
@group(2) @binding(0) var<uniform> oit: OitParams;
@group(2) @binding(1) var oit_bounds: texture_2d<f32>;
@group(2) @binding(2) var oit_coeff0: texture_2d<f32>;
@group(2) @binding(3) var oit_coeff1: texture_2d<f32>;
@group(2) @binding(4) var oit_accumulated: texture_2d<f32>;

const OIT_FULLY_OPAQUE_ALPHA: f32 = 0.99;
const OIT_FULLY_OPAQUE_TRANSMITTANCE: f32 = 0.02;
const OIT_BINS: i32 = 8;
const OIT_LOW_PRECISION_BINS: f32 = 2.0;
const OIT_HIGH_PRECISION_RANGE: f32 = 10.0;
const OIT_ALPHA_CUTOUT: f32 = 0.1;

fn oit_linear_depth(device_depth: f32) -> f32 {
    return oit.projection.y / (device_depth + oit.projection.x);
}
fn oit_absorbance(transmittance: f32) -> f32 {
    return clamp(-log(max(transmittance, 0.0001)), 0.0, 4.0);
}
fn oit_transmittance(absorbance: f32) -> f32 {
    return clamp(exp(-absorbance), 0.0001, 1.0);
}

// Where a fragment sits in its pixel's depth range, 0 to 1. The last
// OIT_LOW_PRECISION_BINS bins stretch over whatever is past the first ten
// blocks, so a thin pane near the eye keeps its resolution in a deep range.
fn oit_normalize_depth(device_depth: f32, pixel: vec2<f32>) -> f32 {
    let bounds = textureLoad(oit_bounds, vec2<i32>(pixel), 0);
    let nearest = -bounds.r;
    let furthest = bounds.g;
    let linear = oit_linear_depth(device_depth);
    let range = max(furthest - nearest, 0.0001);
    let within = clamp(linear - nearest, 0.0, range);
    let fine_share = (f32(OIT_BINS) - OIT_LOW_PRECISION_BINS) / f32(OIT_BINS);
    let fine_range = fine_share * range;
    if fine_range <= OIT_HIGH_PRECISION_RANGE {
        return within / range;
    }
    if within <= OIT_HIGH_PRECISION_RANGE {
        return within / OIT_HIGH_PRECISION_RANGE * fine_share;
    }
    return fine_share + (within - OIT_HIGH_PRECISION_RANGE)
        / (range - OIT_HIGH_PRECISION_RANGE) * (1.0 - fine_share);
}

struct OitCoefficients {
    @location(0) first: vec4<f32>,
    @location(1) second: vec4<f32>,
}
// One fragment's contribution to the transform (summed by additive blending).
fn oit_add_transmittance(transmittance: f32, frag_depth: f32, pixel: vec2<f32>) -> OitCoefficients {
    var c: array<f32, 8>;
    let absorbance = oit_absorbance(transmittance);
    let original = oit_normalize_depth(frag_depth, pixel);
    let depth = original * f32(OIT_BINS - 1) / f32(OIT_BINS);
    c[0] = absorbance * (1.0 - depth);
    // The wavelet tree's leaves start at index 8.
    var tree = i32(floor(original * f32(OIT_BINS - 1))) + OIT_BINS;
    for (var level = 0; level <= 2; level++) {
        let power = 2 - level;
        let parent = tree >> 1u;
        let index_at_parent = f32(parent & ((1 << u32(power)) - 1));
        let right = tree & 1;
        let side = 1 - (right << 1u);
        let width = exp2(-f32(power));
        let height = exp2(f32(power) * 0.5);
        let offset = depth - width * index_at_parent;
        c[parent] = (f32(right) * width + f32(side) * offset) * height * absorbance;
        tree = parent;
    }
    return OitCoefficients(vec4(c[0], c[1], c[2], c[3]), vec4(c[4], c[5], c[6], c[7]));
}

// The absorbance in front of `original` (a normalized depth), without the
// fragment's own contribution `current` that the transform already holds.
fn oit_sample_absorbance(coefficients: array<f32, 8>, original: f32, current: f32) -> f32 {
    var c = coefficients;
    var average = c[0];
    if average == 0.0 {
        return 0.0;
    }
    let in_bins = original * f32(OIT_BINS - 1);
    let depth = in_bins / f32(OIT_BINS);
    average -= current * (1.0 - depth);
    // B: the bin the fragment is in; A: the one before it.
    var tree_b = clamp(i32(floor(in_bins)), 0, OIT_BINS - 1);
    let sample_a_too = tree_b >= 1;
    var tree_a = select(tree_b, tree_b - 1, sample_a_too);
    tree_b += OIT_BINS;
    tree_a += OIT_BINS;
    var sample_b = average;
    var sample_a = select(0.0, average, sample_a_too);
    for (var level = 0; level <= 2; level++) {
        let power = 2 - level;
        let width = exp2(-f32(power));
        let height = exp2(f32(power) * 0.5);
        let parent_b = tree_b >> 1u;
        let right_b = tree_b & 1;
        let side_b = 1 - (right_b << 1u);
        let index_at_parent = f32(parent_b & ((1 << u32(power)) - 1));
        let offset = depth - width * index_at_parent;
        let own = (f32(right_b) * width + f32(side_b) * offset) * height * current;
        let difference_b = c[parent_b] - own;
        sample_b -= height * difference_b * f32(side_b);
        tree_b = parent_b;
        if sample_a_too {
            let parent_a = tree_a >> 1u;
            let difference_a = select(c[parent_a], difference_b, parent_a == parent_b);
            let side_a = 1 - ((tree_a & 1) << 1u);
            sample_a -= height * difference_a * f32(side_a);
            tree_a = parent_a;
        }
    }
    let blend = select(fract(in_bins), 1.0, in_bins >= f32(OIT_BINS));
    return mix(sample_a, sample_b, blend);
}
fn oit_sample_transmittance(pixel: vec2<i32>, depth: f32, current: f32) -> f32 {
    let a = textureLoad(oit_coeff0, pixel, 0);
    let b = textureLoad(oit_coeff1, pixel, 0);
    let c = array<f32, 8>(a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w);
    return oit_transmittance(oit_sample_absorbance(c, depth, oit_absorbance(current)));
}

// The terrain's translucent layer, once per stage.
fn oit_terrain_color(in: TerrainOut) -> vec4<f32> {
    var color = sprite(in.uv, in.layer & 0x7fffu) * in.color;
    color = mix(globals.fog_color * vec4(1.0, 1.0, 1.0, color.a), color, in.visibility);
    if color.a < OIT_ALPHA_CUTOUT { discard; }
    return color;
}
// Stage 1: (-nearest, furthest, nearest device depth, nearest opaque device
// depth), combined by max blending.
@fragment fn fs_oit_bounds(in: TerrainOut) -> @location(0) vec4<f32> {
    let color = oit_terrain_color(in);
    let linear = oit_linear_depth(in.clip.z);
    let opaque = select(0.0, in.clip.z, color.a > OIT_FULLY_OPAQUE_ALPHA);
    return vec4(-linear, linear, in.clip.z, opaque);
}
// Stage 2.
@fragment fn fs_oit_transmittance(in: TerrainOut) -> OitCoefficients {
    let color = oit_terrain_color(in);
    return oit_add_transmittance(1.0 - color.a, in.clip.z, in.clip.xy);
}
// Stage 3: premultiplied colour, weighted by the transmittance in front.
@fragment fn fs_oit_accumulate(in: TerrainOut) -> @location(0) vec4<f32> {
    let color = oit_terrain_color(in);
    let seen = oit_sample_transmittance(vec2<i32>(in.clip.xy),
        oit_normalize_depth(in.clip.z, in.clip.xy), 1.0 - color.a);
    let weighted = vec4(color.rgb * color.a, color.a) * seen;
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    // The fog colour is premultiplied by the same weight.
    let rgb = mix(weighted.rgb, globals.fog_color.rgb * weighted.a, f * globals.fog_color.a);
    return vec4(rgb, weighted.a);
}

struct OitFragment {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}
// After the bounds stage: a fully opaque fragment hides what is behind it, so
// the far bound stops there and its depth goes into the depth buffer.
@fragment fn fs_oit_cull(@builtin(position) pos: vec4<f32>) -> OitFragment {
    let b = textureLoad(oit_bounds, vec2<i32>(pos.xy), 0);
    let opaque = oit_linear_depth(b.a);
    return OitFragment(vec4(b.r, min(b.g, opaque), b.b, b.a), b.a);
}
// The sum over the world, renormalized to the coverage the transform gives.
@fragment fn fs_oit_composite(@builtin(position) pos: vec4<f32>) -> OitFragment {
    let p = vec2<i32>(pos.xy);
    let accumulated = textureLoad(oit_accumulated, p, 0);
    let seen = oit_sample_transmittance(p, 100000.0, 1.0);
    let coverage = 1.0 - select(seen, 0.0, seen < OIT_FULLY_OPAQUE_TRANSMITTANCE);
    // Additive surfaces add light but no coverage.
    if coverage < 0.00001 && dot(accumulated.rgb, vec3(1.0)) < 0.00001 { discard; }
    let scale = select(1.0, coverage / accumulated.a, accumulated.a > 0.00001);
    return OitFragment(vec4(accumulated.rgb * scale, coverage), textureLoad(oit_bounds, p, 0).b);
}
