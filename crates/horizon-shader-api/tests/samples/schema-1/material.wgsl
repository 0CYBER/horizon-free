fn material(input: MaterialInput) -> Surface {
    let lit = input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0);
    return Surface(vec4<f32>(lit.rgb * (0.9 + 0.1 * luminance(lit.rgb)), lit.a));
}
