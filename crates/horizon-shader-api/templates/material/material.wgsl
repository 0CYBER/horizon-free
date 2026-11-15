// The colour of each point of a block's surface, in linear light, before the alpha test and
// fog. Vanilla's own: albedo * tint * vec4(lightmap, 1.0).
// Reference: docs/api/shaders-reference-0.2.md.
override contrast: f32 = 1.2;

fn material(input: MaterialInput) -> Surface {
    let light = pow(input.lightmap, vec3<f32>(contrast));
    return Surface(input.albedo * input.tint * vec4<f32>(light, 1.0));
}
