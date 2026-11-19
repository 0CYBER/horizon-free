// Blocks, as Horizon's terrain shader draws them with HDR on: vanilla's colour, and on a light
// source the texels that shine lifted by their tier's gain, free of the face's shading and the
// light level that would darken them. Vanilla's lit colour comes from `lit_tint`, the vertex
// colour lit at each corner and smoothed over the face as Horizon smooths it: reading the
// terrain's separate looks instead would make the world load again when the pack starts and take
// a third again its memory.
fn material(input: MaterialInput) -> Surface {
    let texel = gamma(input.albedo.rgb);
    // Vanilla's light here under the vertex colour, coloured lights added at each corner.
    let lighting = gamma(input.lit_tint);
    var color = texel * lighting;
    var mark = 1.0;
    let tier = input.light_source_tier;
    let y = brightness(texel);
    // Only texels as bright as their sprite's threshold shine: its bright parts, or all of it.
    // Translucent terrain is never lifted.
    let shines = input.light_source_whole || y >= input.light_source_threshold;
    if frame.hdr == 1u && tier > 0u && !input.translucent && shines {
        let seen = y * (1.0 - fogged(input.position));
        mark = hdr_mark(tier, seen);
        // As it lifts, the texel leaves its face's shading and its light level behind, down to
        // the lightmap at full light.
        let full = gamma(frame.full_lightmap);
        color *= hdr_unshade(lighting, hdr_lift(tier, seen), max(full.r, max(full.g, full.b)));
    }
    return Surface(vec4<f32>(lit(color, mark), input.albedo.a));
}
