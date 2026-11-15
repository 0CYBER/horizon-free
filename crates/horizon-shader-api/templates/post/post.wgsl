// Runs once per pixel of the world's picture, under the hand and the HUD. Colours are linear
// light, 1.0 at paper white; the result's alpha is ignored.
// Reference: docs/api/shaders-reference-0.2.md (the inputs, `frame` and the helpers).
override saturation: f32 = 1.15;

fn post(input: PostInput) -> vec4<f32> {
    let grey = dot(input.color.rgb, frame.luminance);
    let color = mix(vec3<f32>(grey), input.color.rgb, saturation);
    return vec4<f32>(color, 1.0);
}
