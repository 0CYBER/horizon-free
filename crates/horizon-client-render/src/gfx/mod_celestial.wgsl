// The first member of the renderer's entity Globals uniform, and its own atlas and sampler.
struct Globals {
    view_proj: mat4x4<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex fn vs_celestial_layer(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> Out {
    var clip = globals.view_proj * vec4(position, 1.0);
    // Sky geometry does not disappear when the world's render distance shortens.
    clip.z = max(clip.z, 0.0);
    return Out(clip, uv, color);
}

@fragment fn fs_celestial_layer(in: Out) -> @location(0) vec4<f32> {
    return frame_stored(textureSample(atlas, atlas_sampler, in.uv) * in.color);
}
