override vignette: bool = true;
override samples: i32 = 4;

fn post(input: PostInput) -> vec4<f32> {
    let d = distance(input.uv, vec2<f32>(0.5));
    let dark = select(1.0, 1.0 - smoothstep(0.4, 0.9, d) * 0.5, vignette);
    return vec4<f32>(input.color.rgb * dark, input.color.a);
}
