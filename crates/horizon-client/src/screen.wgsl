// Full-screen camera effects (screen_effects.rs): clip-space quads reading
// one sprite rectangle of the shared 2D atlas as if it were its own texture,
// with the sampler vanilla gives it (ReloadableTexture.setSampler: linear
// when `blur`, clamp-to-edge when `clamp`, else nearest and repeat).
@group(0) @binding(1) var atlas: texture_2d<f32>;
// The block sprites' sampler (nearest), and the entity group's lightmap
// sampler (linear) for `blur`. A blurred sprite has a one-texel border in the
// atlas, wrapped as repeat reads it (hud.rs), so the hardware's bilinear
// sample is GL_LINEAR with repeat; clamp-to-edge keeps the sample point half
// a texel inside. Each sampling has its own pipeline (gfx.rs), so a pixel
// costs what it did with nearest and repeat only: one sample.
@group(0) @binding(2) var nearest_sampler: sampler;
@group(0) @binding(4) var linear_sampler: sampler;
override BLUR: bool = false;
override CLAMP: bool = false;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    // With clamp-to-edge, the sprite's first and last texel centres.
    @location(3) @interpolate(flat) edges: vec4<f32>,
}
@vertex fn vs_screen(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>,
    @location(2) rect: vec4<f32>, @location(3) color: vec4<f32>) -> Out {
    var out: Out;
    out.clip = vec4(pos, 0.0, 1.0);
    out.uv = uv;
    out.rect = rect;
    out.color = color;
    let half = 0.5 / vec2<f32>(textureDimensions(atlas));
    out.edges = vec4(rect.xy + half, rect.zw - half);
    return out;
}
fn sample(in: Out) -> vec4<f32> {
    var f = in.uv;
    if !CLAMP {
        // Repeat: one period of the sprite.
        f = fract(f);
    }
    var uv = in.rect.xy + f * (in.rect.zw - in.rect.xy);
    if CLAMP {
        uv = clamp(uv, in.edges.xy, in.edges.zw);
    }
    if BLUR {
        return textureSampleLevel(atlas, linear_sampler, uv, 0.0) * in.color;
    }
    return textureSampleLevel(atlas, nearest_sampler, uv, 0.0) * in.color;
}
@fragment fn fs_screen(in: Out) -> @location(0) vec4<f32> {
    let c = sample(in);
    if c.a <= 0.0 { discard; }
    return c;
}
// RenderPipelines.VIGNETTE blends ZERO, ONE_MINUS_SRC_COLOR: output the
// texture colour; the blend state darkens the frame by it.
@fragment fn fs_vignette(in: Out) -> @location(0) vec4<f32> {
    return vec4(sample(in).rgb, 1.0);
}
