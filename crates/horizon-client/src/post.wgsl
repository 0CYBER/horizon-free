// Menu backdrop: dual-Kawase blur (down and up passes) and a cold colour grade.
struct Post {
    // Half a source texel (u, v), unused, unused.
    texel: vec4<f32>,
    // Multiplier and lift of the grade.
    tint: vec4<f32>,
    lift: vec4<f32>,
    // Desaturation, vignette, unused, unused.
    params: vec4<f32>,
}
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> post: Post;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2(p.x, 1.0 - p.y);
    return out;
}
fn s(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv, 0.0).rgb;
}
@fragment fn fs_down(in: Out) -> @location(0) vec4<f32> {
    let h = post.texel.xy;
    let c = s(in.uv) * 4.0 + s(in.uv - h) + s(in.uv + h) + s(in.uv + vec2(h.x, -h.y)) + s(in.uv - vec2(h.x, -h.y));
    return vec4(c / 8.0, 1.0);
}
fn up(uv: vec2<f32>) -> vec3<f32> {
    let h = post.texel.xy;
    var c = s(uv + vec2(-h.x * 2.0, 0.0)) + s(uv + vec2(h.x * 2.0, 0.0));
    c += s(uv + vec2(0.0, -h.y * 2.0)) + s(uv + vec2(0.0, h.y * 2.0));
    c += (s(uv + vec2(-h.x, h.y)) + s(uv + vec2(h.x, h.y)) + s(uv + vec2(-h.x, -h.y)) + s(uv + vec2(h.x, -h.y))) * 2.0;
    return c / 12.0;
}
@fragment fn fs_up(in: Out) -> @location(0) vec4<f32> {
    return vec4(up(in.uv), 1.0);
}
@fragment fn fs_final(in: Out) -> @location(0) vec4<f32> {
    var c = up(in.uv);
    let l = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(c, vec3(l), post.params.x);
    c = c * post.tint.rgb + post.lift.rgb;
    let d = length((in.uv - 0.5) * vec2(1.25, 1.0));
    c *= 1.0 - post.params.y * smoothstep(0.35, 0.95, d);
    return vec4(clamp(c, vec3(0.0), vec3(1.0)), 1.0);
}
