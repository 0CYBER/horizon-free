// Vanilla's light, with what the stage reads beyond it plain in the picture: translucent terrain
// (water) is half the water mirror (`reflection`) plus what lies behind it (`opaque_color`, by
// `opaque_depth`); solid terrain stores its normal and sky light for the post pass (`set_surface`).
fn material(input: MaterialInput) -> Surface {
    let lit = input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0);
    if input.translucent {
        let texel = vec2<i32>(input.screen * frame.resolution);
        let behind = textureLoad(opaque_color, texel, 0).rgb;
        // Depth is reversed: near blocks are toward 1, the sky 0.
        let near = clamp(textureLoad(opaque_depth, texel, 0) * 50.0, 0.0, 1.0);
        let mirrored = textureSampleLevel(reflection, reflection_sampler, input.screen, 0.0).rgb;
        return Surface(vec4<f32>(mix(lit.rgb, mirrored, 0.5) + behind * 0.3 * near, lit.a));
    }
    set_surface(input.normal, 0.5, input.light.y);
    return Surface(lit);
}
