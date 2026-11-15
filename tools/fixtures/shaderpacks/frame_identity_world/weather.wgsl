// Rain and snow orange, where vanilla's are blue and white, so the renderer's own streaks show.
fn weather(input: WeatherInput) -> vec4<f32> {
    let grey = dot(input.color.rgb, vec3<f32>(0.3, 0.6, 0.1));
    return vec4<f32>(vec3<f32>(1.0, 0.45, 0.1) * grey * 1.5, input.color.a);
}
