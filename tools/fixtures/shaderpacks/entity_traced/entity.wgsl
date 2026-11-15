// Vanilla's light, tinted magenta: the pictures show that the stage drew the entities.
fn entity(input: EntityInput) -> Surface {
    let lit = input.albedo.rgb * input.tint.rgb * input.shade * input.lightmap;
    return Surface(vec4<f32>(lit * vec3<f32>(1.0, 0.2, 1.0), input.albedo.a * input.tint.a));
}
