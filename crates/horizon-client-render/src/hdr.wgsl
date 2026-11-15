// Horizon's HDR output: the frame's light, stored in vanilla's gamma, fitted to the display (the
// tone curve), the HUD laid over it, and the picture written to the swapchain at the paper
// white: HDR10 (BT.2020, PQ) or scRGB (1.0 = 80 nits).
@group(0) @binding(0) var frame: texture_2d<f32>;
// The tone curve's input: the frame as the post effects left it (Hdr::tonemap).
@group(0) @binding(1) var world: texture_2d<f32>;

struct Params {
    // Paper white in scRGB units (nits / 80).
    paper: f32,
    // Peak brightness relative to paper white.
    peak: f32,
    // 1 for an HDR10 swapchain, 0 for scRGB.
    pq: f32,
    // The display's black level in nits; 0 leaves the picture as it is.
    black: f32,
    // The HUD and menus' brightness as a share of paper white.
    ui: f32,
    // 1 where Horizon's look lifted the world, else 0: whether the light ends at the look's
    // scene peak, which the tone curve then maps onto the display's (fs_tonemap).
    look: f32,
    // HDR calibration (hdr_calibration.rs): x its step (0 none, 1 black level,
    // 2 peak, 3 paper white), y the slider in nits, z where its controls start,
    // a share of the height.
    calibration: vec4<f32>,
};
@group(0) @binding(2) var<uniform> params: Params;
// Horizon's logo, 1 inside, 0 outside, for calibration's test pictures.
@group(0) @binding(3) var logo: texture_2d<f32>;
// The float frame with HUD coverage (Hdr::layered): the frame keeps the world as light
// directly from its writers; the HUD draws on a picture of its own (`hud`, fs_keep), and `coverage` says how
// much of the world it hides there (gfx/hud_coverage.rs).
@group(0) @binding(4) var coverage: texture_2d<f32>;
@group(0) @binding(5) var hud: texture_2d<f32>;

// Linear BT.709 to BT.2020 (ITU-R BT.2087), columns first.
const BT709_TO_BT2020 = mat3x3<f32>(
    vec3(0.6274, 0.0691, 0.0164),
    vec3(0.3293, 0.9195, 0.0880),
    vec3(0.0433, 0.0114, 0.8956),
);

// Its inverse, for scRGB's black level lift.
const BT2020_TO_BT709 = mat3x3<f32>(
    vec3(1.66051121, -0.12456141, -0.01816769),
    vec3(-0.58771059, 1.13296051, -0.1005606),
    vec3(-0.07280062, -0.00839911, 1.11872828),
);

// ITU-R BT.2390's black level lift (its EETF without the knee) on the PQ signal
// `e`: black moves up to the display's black level, the peak stays, and the lift
// fades out by the fourth power on the way up, so only the shadows move. A display
// that can't go darker than its black level would crush everything below it. Both
// outputs lift each of BT.2020's channels, as HDR10 carries them: black rises to the
// display's neutral floor, and a shadow shows the same light in HDR10 and scRGB.
fn lift_black(e: vec3<f32>) -> vec3<f32> {
    let top = pq(vec3(params.peak * params.paper * 80.0)).x;
    let n = min(e / top, vec3(1.0));
    return e + pq(vec3(params.black)).x * pow(vec3(1.0) - n, vec3(4.0));
}

// Whether `p` falls on the logo drawn `size` pixels wide around `centre`.
fn on_logo(p: vec2<f32>, centre: vec2<f32>, size: f32) -> bool {
    let uv = (p - centre) / size + 0.5;
    if any(uv < vec2(0.0)) || any(uv >= vec2(1.0)) {
        return false;
    }
    let texels = vec2<f32>(textureDimensions(logo));
    return textureLoad(logo, vec2<i32>(uv * texels), 0).r > 0.5;
}

// A calibration step's test picture at `p`, in nits, the way display calibration
// tools draw them: nothing in it is bright over a large area, so the display
// doesn't dim itself while it is judged.
fn calibration_nits(p: vec2<f32>, size: vec2<f32>) -> f32 {
    let top = size.y * params.calibration.z;
    let centre = vec2(size.x, top) / 2.0;
    let v = params.calibration.y;
    switch u32(params.calibration.x) {
        // Black level: the logo at the slider on 0. Lowered until it disappears,
        // it sits on the darkest the display shows.
        case 1u: {
            return select(0.0, v, on_logo(p, centre, top * 0.4));
        }
        // Peak: a tenth of the screen at 10 000 nits, which the display clips at
        // its brightest, the logo inside at the slider. Raised until it
        // disappears, it sits on that brightest.
        case 2u: {
            let side = sqrt(0.1 * size.x * size.y);
            if all(abs(p - centre) < vec2(side / 2.0)) {
                return select(10000.0, v, on_logo(p, centre, side * 0.7));
            }
            return 0.0;
        }
        // Paper white: a white card at the slider, on a dim grey, to match a
        // sheet of paper held beside the screen.
        default: {
            let card = vec2(top * 0.9, top * 0.6);
            return select(3.0, v, all(abs(p - centre) < card / 2.0));
        }
    }
}

@vertex fn vs_hdr(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

// Calibration: above its controls the test picture, straight in nits, past
// paper white, the lift and the black level.
fn calibrating(pos: vec4<f32>) -> bool {
    let size = vec2<f32>(textureDimensions(frame));
    return params.calibration.x > 0.5 && pos.y < size.y * params.calibration.z;
}

fn calibration_picture(pos: vec4<f32>) -> vec4<f32> {
    let nits = vec3(calibration_nits(pos.xy, vec2<f32>(textureDimensions(frame))));
    if params.pq > 0.5 {
        return vec4(pq(nits), 1.0);
    }
    return vec4(nits / 80.0, 1.0);
}

// On for the HDR10 swapchain alone (Hdr::new): its 10 bits get dither. A screenshot's HDR10 copy
// (Hdr::capture) keeps the exact codes, and scRGB's floats need none.
override pq_dither: bool = false;

// Three even random values in [0, 1) for `at`: PCG's 3D hash (Jarzynski and Olano, "Hash Functions
// for GPU Rendering", JCGT 2020). Integers only, so it is the same on every GPU and at every
// resolution, where hashes built on `fract(sin())` repeat or streak at 4K's coordinates.
// Copyright 2020 Mark Jarzynski & Marc Olano, MIT (github.com/markjarzynski/PCG3D; the license is in
// THIRD_PARTY_NOTICES.md).
fn hash3(at: vec3<u32>) -> vec3<f32> {
    var v = at * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return vec3<f32>(v >> vec3(8u)) / 16777216.0;
}

// The PQ signal `e` with dither for its 10 bits: triangular noise of one code either way, each
// channel's own, so a slow gradient (a clear sky is a few codes tall) shows as grain a code deep
// instead of bands, and its mean is the light meant. The noise is the pixel's alone: no frame
// counter, so a still picture stays still. Within a code of black or of the top it shrinks to
// what fits: the signal never leaves 0..1, and black stays exactly black (an OLED's pixel off).
// The price is in that last code alone: the smaller noise keeps the signal's mean but not the
// rounded one, which leans to the end (a third of a code from it shows as an eighth). Under
// 0.00004 nits at black, that is the accepted cost of a true 0.
fn dithered(e: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    let at = vec2<u32>(pixel);
    let noise = hash3(vec3(at, 0u)) + hash3(vec3(at, 1u)) - 1.0;
    return e + noise * min(min(e, 1.0 - e), vec3(1.0 / 1023.0));
}

// Light in units of paper white at the pixel `pos`, as the swapchain takes it: HDR10 or scRGB,
// the black level lifted.
fn display(light: vec3<f32>, pos: vec4<f32>) -> vec4<f32> {
    let scrgb = light * params.paper;
    if params.pq > 0.5 {
        let e = pq(BT709_TO_BT2020 * scrgb * 80.0);
        let lifted = select(e, lift_black(e), params.black > 0.0);
        if pq_dither {
            return vec4(dithered(lifted, pos.xy), 1.0);
        }
        return vec4(lifted, 1.0);
    }
    if params.black > 0.0 {
        let lifted = pq_nits(lift_black(pq(BT709_TO_BT2020 * scrgb * 80.0)));
        return vec4(BT2020_TO_BT709 * lifted / 80.0, 1.0);
    }
    return vec4(scrgb, 1.0);
}

// The last pass over a frame without the HUD's layer, a `shaders/` pack's (gfx/output.rs): its
// SDR picture, the hand and the HUD with it, at paper white.
@fragment fn fs_hdr(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    if calibrating(pos) {
        return calibration_picture(pos);
    }
    return display(pow(textureLoad(frame, vec2<i32>(pos.xy), 0).rgb, vec3(2.2)), pos);
}

// Hdr::tonemap, after the post effects and before the camera overlays (docs/design/
// hdr-linear-light.md, `[tonemap]`): the light (`world` holds the frame) fitted to the display, in
// gamma, where scaling the light by s scales the stored colour by s^(1/2.2). Light the curve
// leaves alone stays exactly as stored. With no headroom and no look, a clip at white.
//
// The curve is the look's (hdr_look.wgsl `hdr_tone_scale`). Its source is the look's scene peak
// where the look lifted the world, and unknown (0) elsewhere: the look's constant times
// `params.look`, so the peak stays a value the shader compiler meets at run time. As a bare
// constant DX12's compiler folds it into PQ's constants, and a display below the scene's peak
// rounds a few pixels a step apart.
@fragment fn fs_tonemap(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let g = textureLoad(world, vec2<i32>(pos.xy), 0).rgb;
    if params.look == 0.0 && params.peak <= 1.0 {
        return vec4(min(g, vec3(1.0)), 1.0);
    }
    let top = pow(max(max(g.r, max(g.g, g.b)), 0.0), 2.2);
    let s = hdr_tone_scale(top, params.look * HDR_SCENE_PEAK, params.peak, params.paper * 80.0);
    return vec4(select(g * pow(s, 1.0 / 2.2), g, s == 1.0), 1.0);
}

// Hdr::hud_backdrop: the world clipped to white with alpha 0, the frame as the HUD finds it
// with HDR off. The HUD's target is 8-bit, as that frame is.
@fragment fn fs_keep(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let w = textureLoad(frame, vec2<i32>(pos.xy), 0).rgb;
    return vec4(clamp(w, vec3(0.0), vec3(1.0)), 0.0);
}

// The backdrop `fs_keep` left in the HUD's 8-bit target for world `w`: where no HUD draws, the
// target holds exactly this, so the HUD's own part below is exactly 0 there.
fn backdrop(w: vec3<f32>) -> vec3<f32> {
    return round(clamp(w, vec3(0.0), vec3(1.0)) * 255.0) / 255.0;
}

// The HUD over the world in gamma, the space the HUD blends in: its own part, what its backdrop
// B doesn't explain (H = P - (1 - c) B, exact for the layering blends), at UI Brightness, over
// (1 - c) of the world, light above white included. Where the world is at or below white, with
// UI Brightness at paper white, that is the HUD's picture as drawn.
fn layered(at: vec2<i32>) -> vec3<f32> {
    let w = max(textureLoad(frame, at, 0).rgb, vec3(0.0));
    let p = textureLoad(hud, at, 0).rgb;
    let c = textureLoad(coverage, at, 0).r;
    let h = max(p - (1.0 - c) * backdrop(w), vec3(0.0));
    return h * pow(params.ui, 1.0 / 2.2) + (1.0 - c) * w;
}

// Hdr::composite: the picture so far, stored as the frame stores light, for the menu blur.
@fragment fn fs_composite(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4(layered(vec2<i32>(pos.xy)), 1.0);
}

// The world's light in SDR: exactly as stored up to paper white, where surfaces and the sky
// end and SDR's white is, so the screenshot is the SDR game's picture there. Light above it
// (light sources, the sun) is scaled down to white by its brightest channel, which keeps its
// hue. A shoulder would keep more of that light's steps, but white is SDR's top: it would
// have to start below white and darken what isn't lifted.
fn screenshot_world(w: vec3<f32>) -> vec3<f32> {
    let top = max(w.r, max(w.g, w.b));
    // Not `w / max(top, 1.0)`: a GPU's division needn't be exact, even by 1.
    return select(w, w / top, top > 1.0);
}

override screenshot_world_only: bool = false;
// Off where Horizon lifts nothing (HDR Lighting off, or a shader pack's picture): each channel
// is clipped at white as SDR draws it.
override screenshot_keeps_hue: bool = true;

// The ordinary PNG, or the world's icon before the HUD: H as drawn, at 100%, over
// the SDR world's light. This never changes the targets used by HDR output or its copy.
@fragment fn fs_sdr_screenshot(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(pos.xy);
    let w = max(textureLoad(frame, at, 0).rgb, vec3(0.0));
    let mapped = select(min(w, vec3(1.0)), screenshot_world(w), screenshot_keeps_hue);
    if screenshot_world_only {
        return vec4(mapped, 1.0);
    }
    let p = textureLoad(hud, at, 0).rgb;
    let c = textureLoad(coverage, at, 0).r;
    let h = max(p - (1.0 - c) * backdrop(w), vec3(0.0));
    return vec4(h + (1.0 - c) * mapped, 1.0);
}

@fragment fn fs_hdr_layered(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    if calibrating(pos) {
        return calibration_picture(pos);
    }
    return display(min(pow(layered(vec2<i32>(pos.xy)), vec3(2.2)), vec3(params.peak)), pos);
}
