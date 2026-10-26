// The sky's colour along each view ray; the sun, moon, stars and clouds are
// drawn over it. Reference: docs/api/shaders-reference.md.
override glow: f32 = 0.5;

fn sky(input: SkyInput) -> vec4<f32> {
    let toward_sun = max(dot(input.direction, frame.sun_direction), 0.0);
    let warm = vec3<f32>(1.0, 0.6, 0.3) * pow(toward_sun, 8.0) * glow;
    return vec4<f32>(input.color.rgb + warm, input.color.a);
}
