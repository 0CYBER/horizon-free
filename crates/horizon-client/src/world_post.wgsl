// Horizon's world post pass: the world, rendered at the resolution scale,
// copied (bilinear) or anti-aliased with FXAA onto the screen before the
// hand and HUD. Only used when a non-vanilla setting asks for it.
@group(0) @binding(0) var world: texture_2d<f32>;
@group(0) @binding(1) var world_sampler: sampler;

struct PostOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex fn vs_post(@builtin(vertex_index) i: u32) -> PostOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: PostOut;
    out.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2(p.x, 1.0 - p.y);
    return out;
}

@fragment fn fs_copy(in: PostOut) -> @location(0) vec4<f32> {
    // The alpha carries the emissive mark through to HDR output (hdr.wgsl).
    return textureSampleLevel(world, world_sampler, in.uv, 0.0);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

// FXAA 3.11 (console quality): blend along the local edge direction.
@fragment fn fs_fxaa(in: PostOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(world));
    let centre = textureSampleLevel(world, world_sampler, in.uv, 0.0);
    let rgb_m = centre.rgb;
    let nw = luma(textureSampleLevel(world, world_sampler, in.uv + vec2(-0.5, -0.5) * texel, 0.0).rgb);
    let ne = luma(textureSampleLevel(world, world_sampler, in.uv + vec2(0.5, -0.5) * texel, 0.0).rgb);
    let sw = luma(textureSampleLevel(world, world_sampler, in.uv + vec2(-0.5, 0.5) * texel, 0.0).rgb);
    let se = luma(textureSampleLevel(world, world_sampler, in.uv + vec2(0.5, 0.5) * texel, 0.0).rgb);
    let m = luma(rgb_m);
    let lo = min(m, min(min(nw, ne), min(sw, se)));
    let hi = max(m, max(max(nw, ne), max(sw, se)));
    if hi - lo < max(0.0312, hi * 0.125) {
        return centre;
    }
    var dir = vec2(-((nw + ne) - (sw + se)), (nw + sw) - (ne + se));
    let reduce = max((nw + ne + sw + se) * 0.25 * 0.125, 1.0 / 128.0);
    let scale = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * scale, vec2(-8.0), vec2(8.0)) * texel;
    let a = 0.5 * (textureSampleLevel(world, world_sampler, in.uv + dir * (1.0 / 3.0 - 0.5), 0.0).rgb
        + textureSampleLevel(world, world_sampler, in.uv + dir * (2.0 / 3.0 - 0.5), 0.0).rgb);
    let b = a * 0.5 + 0.25 * (textureSampleLevel(world, world_sampler, in.uv - dir * 0.5, 0.0).rgb
        + textureSampleLevel(world, world_sampler, in.uv + dir * 0.5, 0.0).rgb);
    let lb = luma(b);
    if lb < lo || lb > hi {
        return vec4(a, centre.a);
    }
    return vec4(b, centre.a);
}
