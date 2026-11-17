// Vanilla's own terrain light (docs/api/shaders-reference-0.2.md, `Surface`), then the fog.
fn material(input: MaterialInput) -> Surface {
    let lit = input.albedo.rgb * input.tint.rgb * input.lightmap;
    return Surface(fogged(vec4<f32>(lit, input.albedo.a * input.tint.a), input.position));
}
