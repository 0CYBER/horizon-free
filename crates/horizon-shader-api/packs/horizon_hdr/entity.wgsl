// Entities and block entities, as Horizon's entity shader draws them with HDR on: vanilla's
// colour (the reference's own `entity`, coloured lights added to the light), and what shines
// lifted by its tier's gain. They keep their shading: it is their own, not a face's.
fn entity(input: EntityInput) -> Surface {
    let base = gamma(input.albedo.rgb * input.tint.rgb * input.shade);
    let flashed = mix(base, gamma(input.overlay.rgb), input.overlay.a);
    let light = min(gamma(input.lightmap) + gamma(input.colored_light), vec3<f32>(1.0));
    var mark = 1.0;
    let tier = input.light_source_tier;
    if frame.hdr == 1u && tier > 0u && input.albedo.a >= 0.1 {
        // Every texel shines by its own brightness. A layer that fades (the warden's) shines by
        // what is left of it, and fog hides it as it hides everything else.
        let seen = brightness(base) * input.tint.a * (1.0 - fogged(input.position));
        mark = hdr_mark(tier, seen);
    }
    return Surface(vec4<f32>(lit(flashed * light, mark), input.albedo.a * input.tint.a));
}
