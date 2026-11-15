// Horizon's HDR look (docs/design/hdr-linear-light.md, "Horizon's look"): what shines past paper
// white with HDR on and no shader pack, and how brightly. Its numbers and its maths, as plain
// functions of their arguments: nothing here names the renderer (no binding, uniform or
// override), so a shader pack of API 0.2 can list this file as a shared source and draw the
// same look. The renderer's own shaders start with it (gfx/wgsl_prefix.rs `HDR_LIGHT`).
//
// Light is linear, 1.0 at paper white. A colour is in vanilla's gamma: its 2.2 power is its
// light.

// The tiers of light sources, x to w (tiers 1 to 4): low (faint glowers such as magma),
// incandescent (flames and lava), bright (glowstone and campfires) and very bright (the sun). A
// texel lifts by its own brightness before lighting, from none at its tier's START to all of
// it at FULL, and a fully lifted one's light is multiplied by its tier's GAIN, in multiples of
// paper white: the same on every display. Lit surfaces get no gain, however strongly they are
// lit: diffuse white stays at paper white, and the range above it is the light sources' and
// the sun's.
const HDR_LIFT_START = vec4<f32>(0.08, 0.14, 0.14, 0.14);
const HDR_LIFT_FULL = vec4<f32>(0.09, 0.38, 0.38, 0.38);
const HDR_GAIN = vec4<f32>(1.6, 2.4, 2.8, 4.0);
// The scene's peak, in multiples of paper white: just above the brightest light the world
// writes (the sun's disc, white at the very bright tier's 4). The world is mastered for a
// 1000-nit display at ITU-R BT.2408's 203-nit reference white, as HDR10 content usually is:
// there it shows as written, a dimmer display eases its highlights into its peak, and a
// brighter one shows it as at 1000, but for the sun's disc.
const HDR_SCENE_PEAK: f32 = 1000.0 / 203.0;
// How far the sun's disc may pass the scene's peak: on a display above HDR_SCENE_PEAK its white
// is the display's peak, up to this, 2000 nits at a paper white of 203. Nothing else passes the
// scene's peak, so a brighter display shows a brighter sun and the same world.
const HDR_SUN_CAP: f32 = 2000.0 / 203.0;

// A colour's brightness: the mean of luminance and the brightest channel, in linear light.
fn brightness(c: vec3<f32>) -> f32 {
    let l = pow(max(c, vec3(0.0)), vec3(2.2));
    return 0.5 * (dot(l, vec3(0.2126, 0.7152, 0.0722)) + max(l.r, max(l.g, l.b)));
}

// How much a light source's pixel lifts, 0 to 1: its own brightness `y` before lighting through
// its tier's window (1 low .. 4 very bright).
fn hdr_lift(tier: u32, y: f32) -> f32 {
    let t = clamp(tier, 1u, 4u) - 1u;
    return smoothstep(HDR_LIFT_START[t], HDR_LIFT_FULL[t], y);
}

// Whether brightness `y` is past its tier's window: all of its lift.
fn hdr_lifts_fully(tier: u32, y: f32) -> bool {
    return y >= HDR_LIFT_FULL[clamp(tier, 1u, 4u) - 1u];
}

// A pixel's mark, which carries its lift to `hdr_lit`: below 0.75, a band of 0.175 per tier
// (from 0.525 for low down to 0 for very bright) and in it how much the pixel lifts
// (`hdr_lift`), the lower the more. From 0.75 up nothing is lifted; a lit surface's is 1.
fn hdr_mark(tier: u32, y: f32) -> f32 {
    let t = clamp(tier, 1u, 4u) - 1u;
    return 0.175 * f32(3u - t) + 0.165 * (1.0 - hdr_lift(tier, y));
}

// A mark has 8 bits, the precision the tiers' look was tuned at.
fn hdr_quantized_mark(mark: f32) -> f32 {
    return round(saturate(mark) * 255.0) / 255.0;
}

// A pixel's light, and whether a light source's lift is in it.
struct Lit {
    light: vec3<f32>,
    lifted: bool,
};

// The light of a pixel drawn `color` under `mark`, where each tier's fully lifted pixels gain
// `gains`. `on`: whether the look is; off, the pixel's light is its colour's, clipped at white
// as SDR draws it.
fn hdr_mark_light(color: vec3<f32>, mark: f32, on: bool, gains: vec4<f32>) -> Lit {
    let a = hdr_quantized_mark(mark);
    var lifted = false;
    // Light over 1 shows only where the pixel carries no mark (1): a lit surface's own, a
    // coloured light adding past white. Marked pixels clip at white before their lift, so light
    // sources and the sun keep the look they were tuned at.
    let c = select(min(color, vec3(1.0)), color, on && a == 1.0);
    // Gamma 2.2, as SDR displays and the compositors' SDR paths decode it.
    var light = pow(c, vec3(2.2));
    // The band is the tier and where in the band how much (`hdr_mark`).
    if on && a < 0.75 {
        let band = u32(clamp(floor((a + 0.005) / 0.175), 0.0, 3.0));
        let tier = 3u - band;
        let lift = 1.0 - clamp((a - 0.175 * f32(band)) / 0.165, 0.0, 1.0);
        if lift > 0.0 {
            let gain = 1.0 + (gains[tier] - 1.0) * lift;
            light *= max(gain, 1.0);
            lifted = true;
        }
    }
    return Lit(light, lifted);
}

// The light of a pixel drawn `color` (never negative) under `mark` (`hdr_mark`) with the look
// `on`: a light source's lifted by its tier's gain, a lit surface's as drawn.
fn hdr_lit(color: vec3<f32>, mark: f32, on: bool) -> Lit {
    return hdr_mark_light(color, mark, on, HDR_GAIN);
}

// The sun's disc's white, in multiples of paper white, on a display whose peak is `headroom`
// times paper white: the scene's peak, and on a display above it the display's, up to
// HDR_SUN_CAP. The one thing of the display the world's light follows; the tone curve fits the
// rest to it.
fn hdr_sun_peak(headroom: f32) -> f32 {
    return clamp(headroom, HDR_SCENE_PEAK, HDR_SUN_CAP);
}

// The sun's disc's light on that display with the look `on`: its colour `color` (never
// negative), clipped at white, with white at the sun's peak.
fn hdr_sun_lit(color: vec3<f32>, headroom: f32, on: bool) -> Lit {
    return hdr_mark_light(color, 0.0, on, vec4(hdr_sun_peak(headroom)));
}

// A light source's own light doesn't depend on the light that falls on it. What to scale a lit
// texel's colour by, so that as its `lift` rises to 1 the `shade` it was multiplied by (face
// shading, ambient occlusion, the lightmap at its light level) loses its darkening: its brightest
// channel goes to `full`, the lightmap's brightest channel at full light. That is 1 unless
// something dims all light (the Darkness effect, a wither's bar), and a light source keeps that.
// A fully lifted texel then shines the same on every face and at every light level; an unlifted
// one is scaled by exactly 1. The hue stays the shade's on purpose: it is the hue SDR shows for
// that face, and this changes brightness only. Never below 1: coloured lights add past the
// lightmap, so a shade can be brighter than `full`, and lifting doesn't darken.
fn hdr_unshade(shade: vec3<f32>, lift: f32, full: f32) -> f32 {
    return mix(1.0, max(full / max(max(shade.r, max(shade.g, shade.b)), 1e-4), 1.0), lift);
}
