// TFR prepared history (LSR/internal scaling mode): scene color -> prepared color.
// Edge/depth-break metadata is written to a separate rgba8unorm target (filterable,
// storage-capable) to avoid overwriting HDR alpha.
// Scene depth is read directly from texture_depth_2d (e.g. Depth32Float).

struct Params {
    // x,y: texel size in UV; z: edge scale; w: reserved.
    params: vec4<f32>,
};

@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var scene_sampler: sampler;
@group(0) @binding(3) var output_color: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var output_edge: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(5) var<uniform> cfg: Params;

fn depth_at(p: vec2<i32>, size: vec2<i32>) -> f32 {
    let q = clamp(p, vec2<i32>(0), size - 1);
    return textureLoad(scene_depth, q, 0);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(output_color));
    if (i32(gid.x) >= size.x || i32(gid.y) >= size.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) / vec2<f32>(size);
    let color = textureSampleLevel(scene_color, scene_sampler, uv, 0.0);
    let c = depth_at(p, size);
    let dx = abs(c - depth_at(p + vec2<i32>(1, 0), size));
    let dy = abs(c - depth_at(p + vec2<i32>(0, 1), size));
    let edge = clamp(max(dx, dy) * cfg.params.z, 0.0, 1.0);
    textureStore(output_color, p, vec4<f32>(clamp(color.rgb, vec3<f32>(0.0), vec3<f32>(65000.0)), color.a));
    textureStore(output_edge, p, vec4<f32>(edge, edge, edge, 1.0));
}
