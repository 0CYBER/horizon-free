// Full-screen camera effects (screen_effects.rs): clip-space quads reading
// one sprite rectangle of the shared 2D atlas as if it were its own texture,
// with the sampler vanilla gives it (ReloadableTexture.setSampler: linear
// when `blur`, clamp-to-edge when `clamp`, else nearest and repeat).
@group(0) @binding(1) var atlas: texture_2d<f32>;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) sampling: u32,
}
@vertex fn vs_screen(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>,
    @location(2) rect: vec4<f32>, @location(3) color: vec4<f32>,
    @location(4) sampling: u32) -> Out {
    var out: Out;
    out.clip = vec4(pos, 0.0, 1.0);
    out.uv = uv;
    out.rect = rect;
    out.color = color;
    out.sampling = sampling;
    return out;
}
const SAMPLE_BLUR: u32 = 1u;
const SAMPLE_CLAMP: u32 = 2u;
// A texel of the sprite, addressed by repeat or clamp-to-edge. With repeat, `p` is at most one
// texel outside the sprite (`sample` reduces the coordinate first), so wrapping is a select: no
// integer division, which Intel GPUs emulate (it cost 0.85 ms a frame on the vignette on an HD 620).
fn texel(origin: vec2<i32>, size: vec2<i32>, p: vec2<i32>, clamp_edge: bool) -> vec4<f32> {
    var q = select(select(p, p - size, p >= size), p + size, p < vec2(0));
    if clamp_edge { q = clamp(p, vec2(0), size - 1); }
    return textureLoad(atlas, origin + q, 0);
}
fn sample(in: Out) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(atlas));
    let origin = vec2<i32>(round(in.rect.xy * dims));
    let size = max(vec2<i32>(round((in.rect.zw - in.rect.xy) * dims)), vec2(1));
    let clamp_edge = (in.sampling & SAMPLE_CLAMP) != 0u;
    // Repeat: one period of the sprite, so every texel read is at most one outside it.
    let p = select(fract(in.uv), in.uv, clamp_edge) * vec2<f32>(size);
    if (in.sampling & SAMPLE_BLUR) == 0u {
        return texel(origin, size, vec2<i32>(floor(p)), clamp_edge) * in.color;
    }
    // GL_LINEAR: the four texels around the sample point, weighted.
    let c = p - 0.5;
    let i = vec2<i32>(floor(c));
    let t = c - floor(c);
    let a = mix(texel(origin, size, i, clamp_edge), texel(origin, size, i + vec2(1, 0), clamp_edge), t.x);
    let b = mix(texel(origin, size, i + vec2(0, 1), clamp_edge), texel(origin, size, i + vec2(1, 1), clamp_edge), t.x);
    return mix(a, b, t.y) * in.color;
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
