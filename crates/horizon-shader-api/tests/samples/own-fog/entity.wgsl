// Vanilla's own entity light (docs/api/shaders-reference-0.2.md, `entity`), then the fog.
fn entity(input: EntityInput) -> Surface {
    let gamma = vec3<f32>(1.0 / 2.2);
    let base = input.albedo.rgb * input.tint.rgb * input.shade;
    let flashed = pow(mix(pow(base, gamma), pow(input.overlay.rgb, gamma), input.overlay.a), vec3<f32>(2.2));
    let lit = vec4<f32>(flashed * input.lightmap, input.albedo.a * input.tint.a);
    return Surface(fogged(lit, input.position));
}
