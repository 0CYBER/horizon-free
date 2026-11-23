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

// Unrolled Haar wavelet level 1 height: `exp2(0.5) == sqrt(2)`.
const OIT_WAVELET_HEIGHT_1: f32 = 1.41421356;

struct OitCoefficients {
    @location(0) first: vec4<f32>,
    @location(1) second: vec4<f32>,
}

fn oit_lane(v: vec4<f32>, index: i32) -> f32 {
    let lo = select(v.x, v.y, (index & 1) != 0);
    let hi = select(v.z, v.w, (index & 1) != 0);
    return select(lo, hi, (index & 2) != 0);
}

// One fragment's contribution to the transform (summed by additive blending).
// Unrolled over the three constant Haar wavelet levels of `OIT_BINS == 8`
// (widths 0.25, 0.5, 1.0; heights 2.0, sqrt(2), 1.0) so no `exp2` calls or
// dynamically-indexed local array spill into scratch memory.
fn oit_add_transmittance(transmittance: f32, frag_depth: f32, pixel: vec2<f32>) -> OitCoefficients {
    let absorbance = oit_absorbance(transmittance);
    let original = oit_normalize_depth(frag_depth, pixel);
    let in_bins = original * f32(OIT_BINS - 1);
    let depth = in_bins / f32(OIT_BINS);
    let c0 = absorbance * (1.0 - depth);
    let bin = clamp(i32(floor(in_bins)), 0, OIT_BINS - 1);

    // Level 0 (power 2): width 0.25, height 2.0, parent in 4..8 (`second`).
    let p0 = bin >> 1u;
    let r0 = (bin & 1) != 0;
    let offset0 = depth - 0.25 * f32(p0);
    let v0 = select(offset0, 0.25 - offset0, r0) * 2.0 * absorbance;

    // Level 1 (power 1): width 0.5, height sqrt(2), parent in 2..4 (`first.zw`).
    let p1 = bin >> 2u;
    let r1 = (p0 & 1) != 0;
    let offset1 = depth - select(0.0, 0.5, p1 != 0);
    let v1 = select(offset1, 0.5 - offset1, r1) * OIT_WAVELET_HEIGHT_1 * absorbance;

    // Level 2 (power 0): width 1.0, height 1.0, parent 1 (`first.y`).
    let r2 = p1 != 0;
    let v2 = select(depth, 1.0 - depth, r2) * absorbance;

    let first = vec4<f32>(c0, v2, select(v1, 0.0, r2), select(0.0, v1, r2));
    let second = vec4<f32>(
        select(0.0, v0, p0 == 0),
        select(0.0, v0, p0 == 1),
        select(0.0, v0, p0 == 2),
        select(0.0, v0, p0 == 3),
    );
    return OitCoefficients(first, second);
}

// The absorbance in front of `original` (a normalized depth), without the
// fragment's own contribution `current` that the transform already holds.
fn oit_sample_absorbance(c0: vec4<f32>, c1: vec4<f32>, original: f32, current: f32) -> f32 {
    var average = c0.x;
    if average == 0.0 {
        return 0.0;
    }
    let in_bins = original * f32(OIT_BINS - 1);
    let depth = in_bins / f32(OIT_BINS);
    average -= current * (1.0 - depth);
    // B: the bin the fragment is in; A: the one before it.
    let bin_b = clamp(i32(floor(in_bins)), 0, OIT_BINS - 1);
    let sample_a_too = bin_b >= 1;
    let bin_a = select(bin_b, bin_b - 1, sample_a_too);
    var sample_b = average;
    var sample_a = select(0.0, average, sample_a_too);

    // Level 0 (power 2): width 0.25, height 2.0, parents in 4..8 (`c1`).
    let p0_b = bin_b >> 1u;
    let r0_b = (bin_b & 1) != 0;
    let offset0 = depth - 0.25 * f32(p0_b);
    let own0 = select(offset0, 0.25 - offset0, r0_b) * 2.0 * current;
    let diff0_b = oit_lane(c1, p0_b) - own0;
    let step0_b = 2.0 * diff0_b;
    sample_b -= select(step0_b, -step0_b, r0_b);
    if sample_a_too {
        let p0_a = bin_a >> 1u;
        let diff0_a = select(oit_lane(c1, p0_a), diff0_b, p0_a == p0_b);
        let step0_a = 2.0 * diff0_a;
        sample_a -= select(step0_a, -step0_a, (bin_a & 1) != 0);
    }

    // Level 1 (power 1): width 0.5, height sqrt(2), parents in 2..4 (`c0.zw`).
    let p1_b = bin_b >> 2u;
    let r1_b = (p0_b & 1) != 0;
    let offset1 = depth - select(0.0, 0.5, p1_b != 0);
    let own1 = select(offset1, 0.5 - offset1, r1_b) * OIT_WAVELET_HEIGHT_1 * current;
    let diff1_b = select(c0.z, c0.w, p1_b != 0) - own1;
    let step1_b = OIT_WAVELET_HEIGHT_1 * diff1_b;
    sample_b -= select(step1_b, -step1_b, r1_b);
    if sample_a_too {
        let p1_a = bin_a >> 2u;
        let diff1_a = select(select(c0.z, c0.w, p1_a != 0), diff1_b, p1_a == p1_b);
        let step1_a = OIT_WAVELET_HEIGHT_1 * diff1_a;
        sample_a -= select(step1_a, -step1_a, ((bin_a >> 1u) & 1) != 0);
    }

    // Level 2 (power 0): width 1.0, height 1.0, parent 1 (`c0.y`).
    let r2_b = p1_b != 0;
    let own2 = select(depth, 1.0 - depth, r2_b) * current;
    let diff2_b = c0.y - own2;
    sample_b -= select(diff2_b, -diff2_b, r2_b);
    if sample_a_too {
        sample_a -= select(diff2_b, -diff2_b, (bin_a >> 2u) != 0);
    }

    let blend = select(fract(in_bins), 1.0, in_bins >= f32(OIT_BINS));
    return mix(sample_a, sample_b, blend);
}

fn oit_sample_transmittance(pixel: vec2<i32>, depth: f32, current: f32) -> f32 {
    let a = textureLoad(oit_coeff0, pixel, 0);
    let b = textureLoad(oit_coeff1, pixel, 0);
    return oit_transmittance(oit_sample_absorbance(a, b, depth, oit_absorbance(current)));
}

// The three stages' ends for a pipeline whose fragment colour is computed as
// it is without improved transparency (vanilla's `executeAlphaOnlyPhase` and
// `sampleColorForAccumulation`). `color` is the classic result, fogged:
// weighting a fogged colour equals fogging the weighted one, since the fog
// is a mix and vanilla premultiplies the fog colour by the same alpha.
fn oit_bounds_fragment(alpha: f32, clip: vec4<f32>) -> vec4<f32> {
    if alpha < 0.01 { discard; }
    let linear = oit_linear_depth(clip.z);
    let opaque = select(0.0, clip.z, alpha > OIT_FULLY_OPAQUE_ALPHA);
    return vec4(-linear, linear, clip.z, opaque);
}
fn oit_transmittance_fragment(alpha: f32, clip: vec4<f32>) -> OitCoefficients {
    return oit_add_transmittance(1.0 - alpha, clip.z, clip.xy);
}
fn oit_accumulate_fragment(color: vec4<f32>, clip: vec4<f32>) -> vec4<f32> {
    let seen = oit_sample_transmittance(vec2<i32>(clip.xy),
        oit_normalize_depth(clip.z, clip.xy), 1.0 - color.a);
    return vec4(color.rgb * color.a, color.a) * seen;
}
// `OIT_ADDITIVE`: no coverage and nothing in the transmittance. The colour
// is not fogged yet: the fog colour is premultiplied by the alpha, which is
// zero, so the fog fades it out instead of tinting it.
fn oit_accumulate_additive(color: vec4<f32>, fog: f32, clip: vec4<f32>) -> vec4<f32> {
    let seen = oit_sample_transmittance(vec2<i32>(clip.xy),
        oit_normalize_depth(clip.z, clip.xy), 1.0);
    return vec4(color.rgb * color.a * (1.0 - fog), 0.0) * seen;
}
