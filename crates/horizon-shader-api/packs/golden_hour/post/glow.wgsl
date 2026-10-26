// A soft halo around bright pixels: twelve taps on two rings, scaled with
// the picture's height so it looks the same at any resolution.
override glow: f32 = 0.35;

fn bright(c: vec3<f32>) -> vec3<f32> {
    return c * smoothstep(0.7, 1.0, luminance(c));
}

fn post(input: PostInput) -> vec4<f32> {
    if glow <= 0.0 {
        return input.color;
    }
    let step = frame.resolution.y / 720.0 / frame.resolution;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 12; i++) {
        let angle = f32(i) * 0.5235988;
        let radius = select(3.0, 7.0, (i & 1) == 1);
        let offset = vec2<f32>(cos(angle), sin(angle)) * radius * step;
        sum += bright(textureSampleLevel(previous, linear_sampler, input.uv + offset, 0.0).rgb);
    }
    return vec4<f32>(input.color.rgb + sum * (glow / 6.0), input.color.a);
}
