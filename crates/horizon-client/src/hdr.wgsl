// Horizon's HDR output pass: the frame, drawn as without HDR in vanilla's gamma
// space, decoded to linear light at the paper white and written to the swapchain:
// HDR10 (BT.2020, PQ) or scRGB (1.0 = 80 nits). Bright pixels of light sources that still show the
// world as it was before the hand and HUD are expanded towards the peak.
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var world: texture_2d<f32>;

struct Params {
    // Paper white in scRGB units (nits / 80).
    paper: f32,
    // Peak brightness relative to paper white; 1 expands nothing.
    peak: f32,
    // 1 when this frame drew a world.
    world: f32,
    // 1 paints expanded pixels magenta (HORIZON_HDR_DEBUG=1).
    debug: f32,
    // 1 for an HDR10 swapchain, 0 for scRGB.
    pq: f32,
};
@group(0) @binding(2) var<uniform> params: Params;

// Light sources are lifted evenly rather than by a highlight curve: vanilla's
// textures were drawn for SDR, so a curve on their brightest pixels blows out
// near-white spots (glowstone) and leaves the rest (lava, a jack o'lantern's
// face) nearly as it was. Pixels brighter than LIFT_FULL get STRENGTH of the
// headroom, dimmer ones less down to none at LIFT_START (lava's crust, a
// lantern's metal), and no channel goes past the peak. Brightness is the mean
// of luminance and the brightest channel, so saturated light counts as bright.
const LIFT_START: f32 = 0.18;
const LIFT_FULL: f32 = 0.42;
const STRENGTH: f32 = 0.7;

// Whether `c` still shows the world pixel `w`: the same, or darkened evenly
// across its channels, as the vignette and other screen-wide tints darken it.
// The hand, HUD and text replace pixels instead, so they never pass. The
// channels are held to one factor, the ratio of their sums, so a channel at 0
// stays 0; a step of 8-bit rounding is allowed on each.
fn shows_world(c: vec3<f32>, w: vec3<f32>) -> bool {
    if all(c == w) {
        return true;
    }
    let f = (c.r + c.g + c.b) / max(w.r + w.g + w.b, 1.0 / 255.0);
    return f <= 1.01 && f > 0.5 && all(abs(c - f * w) <= 0.03 * w + 1.0 / 255.0);
}

// Linear BT.709 to BT.2020 (ITU-R BT.2087), columns first.
const BT709_TO_BT2020 = mat3x3<f32>(
    vec3(0.6274, 0.0691, 0.0164),
    vec3(0.3293, 0.9195, 0.0880),
    vec3(0.0433, 0.0114, 0.8956),
);

// SMPTE ST 2084: nits (up to 10000) to the PQ signal.
fn pq(nits: vec3<f32>) -> vec3<f32> {
    let y = pow(clamp(nits / 10000.0, vec3(0.0), vec3(1.0)), vec3(0.1593017578125));
    return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), vec3(78.84375));
}

@vertex fn vs_hdr(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment fn fs_hdr(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(pos.xy);
    let c = textureLoad(frame, at, 0).rgb;
    // Gamma 2.2, as SDR displays and the compositors' SDR paths decode it.
    var light = pow(max(c, vec3(0.0)), vec3(2.2));
    let w = textureLoad(world, at, 0);
    // The world's alpha is 0 on light-emitting blocks, the sun and the moon
    // (terrain.wgsl, the celestial pipeline): only those shine.
    if params.world > 0.5 && params.peak > 1.0 && w.a < 0.75 && shows_world(c, w.rgb) {
        // Brightness of the world pixel, before the vignette darkens the frame
        // toward its edges: a light source shines the same anywhere on screen.
        let wl = pow(max(w.rgb, vec3(0.0)), vec3(2.2));
        let y = 0.5 * (dot(wl, vec3(0.2126, 0.7152, 0.0722)) + max(wl.r, max(wl.g, wl.b)));
        let lift = smoothstep(LIFT_START, LIFT_FULL, y);
        if lift > 0.0 {
            let top = max(light.r, max(light.g, light.b));
            let gain = min(1.0 + (params.peak - 1.0) * STRENGTH * lift, params.peak / max(top, 1e-4));
            light *= max(gain, 1.0);
            if params.debug > 0.5 {
                light = mix(light, vec3(gain, 0.0, gain), 0.6);
            }
        }
    }
    let scrgb = light * params.paper;
    if params.pq > 0.5 {
        return vec4(pq(BT709_TO_BT2020 * scrgb * 80.0), 1.0);
    }
    return vec4(scrgb, 1.0);
}
