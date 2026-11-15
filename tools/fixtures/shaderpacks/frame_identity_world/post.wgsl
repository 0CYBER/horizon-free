// Solid terrain tinted by the normal and sky light `set_surface` stored, and a slight grade over
// everything, so the pack shows in every scene (the End's sky, which no other stage reaches).
fn post(input: PostInput) -> vec4<f32> {
    var color = input.color.rgb * vec3<f32>(1.0, 0.96, 1.04);
    let surface = surface_at(vec2<i32>(input.pixel));
    if surface.present {
        color *= vec3<f32>(0.8) + 0.3 * abs(surface.normal) * (0.5 + 0.5 * surface.data);
    }
    return vec4<f32>(color, 1.0);
}
