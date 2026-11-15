// A little cooler than vanilla's, so that a picture shows the pack ran.
fn sky(input: SkyInput) -> vec4<f32> {
    return vec4<f32>(input.color.rgb * vec3<f32>(0.9, 1.0, 1.1), input.color.a);
}
