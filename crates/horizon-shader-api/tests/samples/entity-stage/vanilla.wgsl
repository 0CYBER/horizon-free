// Horizon's own draw of an entity as a stage: vanilla's own (docs/api/shaders-reference-0.2.md),
// with the frame's coloured lights added to the light colour in vanilla's gamma, as Horizon adds
// them. A pack with it is drawn as the pack without the stage, up to the rounding of the decode
// and encode around the stage.
fn entity(input: EntityInput) -> Surface {
    let gamma = vec3<f32>(1.0 / 2.2);
    let base = input.albedo.rgb * input.tint.rgb * input.shade;
    let flashed = pow(mix(pow(base, gamma), pow(input.overlay.rgb, gamma), input.overlay.a), vec3<f32>(2.2));
    let light = pow(min(pow(input.lightmap, gamma) + pow(input.colored_light, gamma), vec3<f32>(1.0)), vec3<f32>(2.2));
    return Surface(vec4<f32>(flashed * light, input.albedo.a * input.tint.a));
}
