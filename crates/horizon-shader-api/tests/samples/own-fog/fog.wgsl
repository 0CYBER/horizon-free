// Vanilla's fog over the lit colour `c` at the camera-relative `position`: mixed in vanilla's
// gamma (the 2.2 power), as Horizon mixes its own, alpha kept.
fn fogged(c: vec4<f32>, position: vec3<f32>) -> vec4<f32> {
    let gamma = vec3<f32>(1.0 / 2.2);
    let mixed = mix(pow(c.rgb, gamma), pow(frame.fog_color, gamma), vanilla_fog(position));
    return vec4<f32>(pow(mixed, vec3<f32>(2.2)), c.a);
}
