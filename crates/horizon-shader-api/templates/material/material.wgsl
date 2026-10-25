// The colour of each point of a block's surface, before the alpha test and
// fog. Vanilla's own: albedo * tint * vec4(lightmap, 1.0).
// Reference: docs/api/shaders-reference.md.
override contrast: f32 = 1.2;

fn material(input: MaterialInput) -> Surface {
    let light = pow(input.lightmap, vec3<f32>(contrast));
    return Surface(input.albedo * input.tint * vec4<f32>(light, 1.0));
}
