// Water that mirrors what a ray off its surface hits: the terrain's colour where the ray meets
// some, the sky's where it doesn't. It shows how `trace` is called from the material stage, not
// how water should look.

// How much of the water's colour the reflection takes the place of.
const MIRROR: f32 = 0.5;
// How far a reflected ray looks, in blocks.
const REACH: f32 = 64.0;

fn material(input: MaterialInput) -> Surface {
    // Vanilla's own colour: the texture, the tint, the light.
    let lit = input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0);
    // Water alone traces: solid terrain is a post pass's to light, once a pixel.
    if !input.translucent || input.fluid != 1u || frame.has_ray_tracing == 0u {
        return Surface(lit);
    }
    let away = reflect(normalize(input.position), input.normal);
    let hit = trace(input.position + input.normal * 0.01, away, REACH);
    var seen = frame.sky_color.rgb;
    if hit.distance >= 0.0 {
        seen = hit.albedo.rgb * hit.tint.rgb;
    }
    // In the water's own light, so it doesn't glow at night.
    return Surface(vec4<f32>(mix(lit.rgb, seen * input.lightmap, MIRROR), lit.a));
}
