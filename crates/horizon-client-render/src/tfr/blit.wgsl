// TFR real-path blit/scale: sample source with linear filtering into output storage.
// Used for real frames and fallbacks where source and output sizes may differ (e.g. LSR/upscale).

@group(0) @binding(0) var blit_src: texture_2d<f32>;
@group(0) @binding(1) var blit_sampler: sampler;
@group(0) @binding(2) var blit_dst: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(blit_dst));
    if (i32(gid.x) >= size.x || i32(gid.y) >= size.y) {
        return;
    }
    let uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) / vec2<f32>(size);
    let c = textureSampleLevel(blit_src, blit_sampler, uv, 0.0);
    textureStore(blit_dst, vec2<i32>(i32(gid.x), i32(gid.y)), c);
}
