// Copy Depth32Float (texture_depth_2d) into sampleable R32Float storage texture.
// Horizon uses reversed-Z (near=1, far/sky~0); values are copied as-is.

@group(0) @binding(0) var depth_src: texture_depth_2d;
@group(0) @binding(1) var depth_dst: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(depth_dst));
    if (i32(gid.x) >= size.x || i32(gid.y) >= size.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let d = textureLoad(depth_src, p, 0);
    textureStore(depth_dst, p, vec4<f32>(d, 0.0, 0.0, 1.0));
}
