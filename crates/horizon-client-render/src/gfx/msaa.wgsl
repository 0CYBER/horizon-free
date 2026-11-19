// Terrain is multisampled before entities, translucency and shader-pack post effects.
// HDR's alpha is a light code, not coverage: it comes from one sample (`kept`) instead of
// averaging codes into a different light tier, as do the depth and the integer surface data.

// The frame holds linear light (`FrameConfiguration::linear_light`): its samples average
// tone-weighted (docs/hdr-passes.md, rule 3), or a sample far above paper white would own the
// edge. Gamma light averages as stored.
override LINEAR: bool = false;

@vertex fn vertex(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let p = vec2(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

@group(0) @binding(0) var seed_color: texture_2d<f32>;
@fragment fn seed(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(seed_color, vec2<i32>(p.xy), 0);
}

@group(1) @binding(0) var samples: texture_multisampled_2d<f32>;
@group(1) @binding(1) var depths: texture_multisampled_2d<f32>;
@group(1) @binding(2) var surfaces: texture_multisampled_2d<u32>;

fn largest(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b));
}

struct Resolved {
    color: vec4<f32>,
    depth: f32,
    surface: vec4<u32>,
}
// The sample the pixel keeps: the second-nearest of its four depths (reverse-Z, a tie to the
// lower index), so a surface covering half the pixel or more owns it, as the single-sample
// frame's pixel centre would. The nearest one would give an edge touching a single sample the
// whole pixel: water, glass, clouds and entities drawn after the resolve fail their depth test
// there, and thin strips of distant water vanish behind the bank.
fn kept(p: vec2<i32>) -> i32 {
    let d0 = textureLoad(depths, p, 0).r;
    let d1 = textureLoad(depths, p, 1).r;
    let d2 = textureLoad(depths, p, 2).r;
    let d3 = textureLoad(depths, p, 3).r;
    // Each sample's rank, nearest first.
    let r0 = u32(d1 > d0) + u32(d2 > d0) + u32(d3 > d0);
    let r1 = u32(d0 >= d1) + u32(d2 > d1) + u32(d3 > d1);
    let r2 = u32(d0 >= d2) + u32(d1 >= d2) + u32(d3 > d2);
    return select(select(select(3, 2, r2 == 1u), 1, r1 == 1u), 0, r0 == 1u);
}
fn resolve(p: vec2<i32>) -> Resolved {
    var rgb = vec3(0.0);
    for (var i = 0; i < 4; i++) {
        let c = textureLoad(samples, p, i).rgb;
        rgb += select(c, c / (1.0 + largest(c)), LINEAR);
    }
    rgb *= 0.25;
    if LINEAR {
        // Each weighted sample stays below 1, so their mean does too. Near half float's largest
        // value the inverse can overshoot it, which the target would store as infinity
        // (docs/hdr-passes.md, rule 5).
        rgb = min(rgb / max(1.0 - largest(rgb), 1e-5), vec3(65504.0));
    }
    let chosen = kept(p);
    let depth = textureLoad(depths, p, chosen).r;
    var surface = textureLoad(surfaces, p, chosen);
    if surface.w != 0u {
        surface.w = bitcast<u32>(depth);
    }
    return Resolved(vec4(rgb, textureLoad(samples, p, chosen).a), depth, surface);
}
struct ColorDepth {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}
@fragment fn resolve_color(@builtin(position) p: vec4<f32>) -> ColorDepth {
    let result = resolve(vec2<i32>(p.xy));
    return ColorDepth(result.color, result.depth);
}
struct ColorDepthSurface {
    @location(0) color: vec4<f32>,
    @location(1) surface: vec4<u32>,
    @builtin(frag_depth) depth: f32,
}
@fragment fn resolve_surface(@builtin(position) p: vec4<f32>) -> ColorDepthSurface {
    let result = resolve(vec2<i32>(p.xy));
    return ColorDepthSurface(result.color, result.surface, result.depth);
}
