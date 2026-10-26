override saturation: f32 = 1.1;
override tonemap: u32 = 1u;

fn post(input: PostInput) -> vec4<f32> {
    let c = input.color.rgb;
    let graded = mix(vec3<f32>(luminance(c)), c, saturation);
    return vec4<f32>(select(graded, graded / (graded + 0.5) * 1.5, tonemap > 0u), input.color.a);
}
