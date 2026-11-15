override HDR_WRITE_LIGHT: bool = false;
// The frame holds linear light: a shader API 0.2 pack's `[frame] blend = "linear"`.
override FRAME_LINEAR: bool = false;

// What the frame stores of `c`, a colour in vanilla's gamma: `c`, or its light (vanilla's 2.2
// power) where the frame holds linear light, so what blends over it mixes as light does.
fn frame_stored(c: vec4<f32>) -> vec4<f32> {
    if FRAME_LINEAR {
        return vec4(pow(max(c.rgb, vec3(0.0)), vec3(2.2)), c.a);
    }
    return c;
}
// `frame_stored` undone: the stored colour `c` in vanilla's gamma.
fn frame_gamma(c: vec4<f32>) -> vec4<f32> {
    if FRAME_LINEAR {
        return vec4(pow(max(c.rgb, vec3(0.0)), vec3(1.0 / 2.2)), c.a);
    }
    return c;
}

// What the float frame stores of a pixel's light `lit` (hdr_look.wgsl): the light in vanilla's
// gamma. The tone curve eases it into the display's peak afterwards (fs_tonemap).
//
// `settings` (Hdr::draw_params): x the display's headroom, its peak over paper white, which the
// sun's disc alone reads (`hdr_written_sun`); z whether Horizon's look lifts the world (the Look
// setting, a world drawn); w debug; y is unused.
fn hdr_stored(lit: Lit, settings: vec4<f32>) -> vec3<f32> {
    var light = lit.light;
    // HORIZON_HDR_DEBUG=1: what isn't lifted turns grey, what is keeps its
    // colour with a magenta tint, so a lift shows on any colour.
    if settings.w > 0.5 && settings.z > 0.5 {
        if lit.lifted {
            light *= vec3(1.0, 0.6, 1.0);
        } else {
            light = vec3(dot(light, vec3(0.2126, 0.7152, 0.0722)));
        }
    }
    return pow(light, vec3(1.0 / 2.2));
}

// The world's writers' light: the colour `color` they drew under its mark (hdr_look.wgsl
// `hdr_mark`; 1 lifts nothing), the same on every display.
fn hdr_written(color: vec3<f32>, mark: f32, settings: vec4<f32>) -> vec3<f32> {
    return hdr_stored(hdr_lit(max(color, vec3(0.0)), mark, settings.z > 0.5), settings);
}

// The sun's disc's light (entity.wgsl's sun pass), its white at the sun's peak on this display.
fn hdr_written_sun(color: vec3<f32>, settings: vec4<f32>) -> vec3<f32> {
    return hdr_stored(hdr_sun_lit(max(color, vec3(0.0)), settings.x, settings.z > 0.5), settings);
}
