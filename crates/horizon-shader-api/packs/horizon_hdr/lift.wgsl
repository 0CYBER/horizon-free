// What the material and entity stages share. The look (hdr_look.wgsl) works on colours in
// vanilla's gamma, as Horizon's own shaders hand it them; a stage's colours are linear light.

// A stage's linear light as the colour vanilla draws.
fn gamma(light: vec3<f32>) -> vec3<f32> {
    return pow(max(light, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

// How much of the fog colour Horizon mixes over a point at the camera-relative `position`, 0..1:
// the larger of the environment's fog along the straight line and the render distance's along
// the larger of the horizontal distance and the height difference. Fog hides a light source as
// it hides everything else, so its lift is worked out from what the fog leaves of it.
fn fogged(position: vec3<f32>) -> f32 {
    let d = vec2<f32>(length(position), max(length(position.xz), abs(position.y)));
    let start = vec2<f32>(frame.fog_environment.x, frame.fog_render.x);
    let end = vec2<f32>(frame.fog_environment.y, frame.fog_render.y);
    let ramp = select((d - start) / (end - start), vec2<f32>(1.0), d >= end);
    let f = select(ramp, vec2<f32>(0.0), d <= start);
    return max(f.x, f.y);
}

// The light of a pixel drawn `color` under `mark` (`hdr_mark`; 1 lifts nothing). In SDR nothing
// is lifted, and the pixel is vanilla's.
fn lit(color: vec3<f32>, mark: f32) -> vec3<f32> {
    return hdr_lit(color, mark, frame.hdr == 1u).light;
}
