// Full-screen camera effects (screen_effects.rs): clip-space quads sampling
// the shared 2D atlas with repeat inside each sprite rectangle.
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
}
@vertex fn vs_screen(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>,
    @location(2) rect: vec4<f32>, @location(3) color: vec4<f32>) -> Out {
    var out: Out;
    out.clip = vec4(pos, 0.0, 1.0);
    out.uv = uv;
    out.rect = rect;
    out.color = color;
    return out;
}
fn sample(in: Out) -> vec4<f32> {
    let f = fract(in.uv);
    let uv = in.rect.xy + f * (in.rect.zw - in.rect.xy);
    return textureSampleLevel(atlas, atlas_sampler, uv, 0.0) * in.color;
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
