// Stage 2 (Composite): Screen-space water reflections (SSR), Fresnel Schlick,
// and Beer-Lambert underwater depth absorption using scene_depth vs opaque_depth.
override water_ssr: bool = true;

@fragment fn fs_main(in: StageIn) -> @location(0) vec4<f32> {
    var color = textureSampleLevel(previous, linear_sampler, in.uv, 0.0).rgb;
    let surface_depth = depth_at(in.uv);
    let floor_depth = opaque_depth_at(in.uv);
    let mat = material_at(in.uv);

    // Translucent water/glass layer exists where surface_depth is closer than floor_depth
    // (reversed depth: closer = larger value) or where the G-buffer water mask is set.
    let is_translucent = (surface_depth > floor_depth + 1e-5) || (mat.b > 0.5);
    if !water_ssr || is_sky(surface_depth) || !is_translucent {
        return vec4(color, 1.0);
    }

    let water_pos = world_offset(in.uv, surface_depth);
    let floor_pos = world_offset(in.uv, max(floor_depth, 1e-6));
    let water_column = clamp(length(floor_pos - water_pos), 0.0, 32.0);

    // Beer-Lambert depth absorption in deep water.
    let absorption = exp(-vec3(0.22, 0.07, 0.03) * water_column * 0.35);
    color *= mix(vec3(1.0), absorption, clamp(water_column * 0.08, 0.0, 0.65));

    // Wave normal perturbation and Fresnel reflection.
    let world_xz = (water_pos.xz + frame.camera_position.xz) * 0.08;
    let wave_uv = world_xz + vec2(frame.time.x * 0.015, -frame.time.x * 0.011);
    let noise_sample = textureSampleLevel(noise_texture, repeat_sampler, wave_uv, 0.0).rgb * 2.0 - 1.0;
    let normal = normalize(vec3(noise_sample.x * 0.06, 1.0, noise_sample.y * 0.06));
    let view_dir = normalize(-water_pos);
    let refl_dir = reflect(-view_dir, normal);
    let fresnel = clamp(0.04 + 0.96 * pow(1.0 - max(dot(view_dir, normal), 0.0), 4.0), 0.0, 0.85);

    // Ray-march 8 steps in screen space along the reflection ray.
    var refl_color = srgb_to_linear(frame.sky_color.rgb);
    let sun_spec = pow(max(dot(refl_dir, frame.sun.xyz), 0.0), 96.0) * 2.5 * (1.0 - frame.weather.x);
    refl_color += vec3(1.0, 0.9, 0.7) * sun_spec;

    for (var step = 1u; step <= 8u; step++) {
        let march_pos = water_pos + refl_dir * (f32(step) * 2.2);
        let clip = frame.projection * (frame.view * vec4(march_pos, 1.0));
        if clip.w > 0.0 {
            let ndc = clip.xyz / clip.w;
            let sample_uv = vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
            if all(sample_uv >= vec2(0.01)) && all(sample_uv <= vec2(0.99)) {
                let scene_d = opaque_depth_at(sample_uv);
                if !is_sky(scene_d) && ndc.z <= scene_d && (scene_d - ndc.z) < 0.08 {
                    refl_color = textureSampleLevel(previous, linear_sampler, sample_uv, 0.0).rgb;
                    break;
                }
            }
        }
    }

    color = mix(color, refl_color, fresnel * 0.55);
    return vec4(color, 1.0);
}
