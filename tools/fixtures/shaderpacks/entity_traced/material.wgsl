// Translucent terrain mixes in the colour a reflected ray hits, so the material's pipelines are
// built against the ray scene where the device traces.
fn material(input: MaterialInput) -> Surface {
    let lit = input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0);
    if !input.translucent || frame.has_ray_tracing == 0u {
        return Surface(lit);
    }
    let hit = trace(input.position + input.normal * 0.01, reflect(normalize(input.position), input.normal), 64.0);
    var seen = frame.sky_color.rgb;
    if hit.distance >= 0.0 {
        seen = hit.albedo.rgb * hit.tint.rgb;
    }
    return Surface(vec4<f32>(mix(lit.rgb, seen * input.lightmap, 0.5), lit.a));
}
