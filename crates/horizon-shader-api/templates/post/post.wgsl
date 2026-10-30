// Runs once per pixel of the world's picture, under the hand and the HUD.
// Reference: docs/api/shaders-reference.md (the inputs, `frame` and the helpers).
override saturation: f32 = 1.15;

fn post(input: PostInput) -> vec4<f32> {
    let grey = dot(input.color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let color = mix(vec3<f32>(grey), input.color.rgb, saturation);
    return vec4<f32>(color, input.color.a);
}
