// Stage 1: G-buffer normals, 8-tap screen-space ambient
// occlusion, directional sun/moon PCF shadow mapping, and rain puddle wetness.
override shadows: bool = true;
override shadow_strength: f32 = 0.85;
override ao_strength: f32 = 0.75;

fn compute_ao(uv: vec2<f32>, depth: f32, view_pos: vec3<f32>) -> f32 {
    if ao_strength <= 0.001 {
        return 1.0;
    }
    let step_uv = frame.resolution.zw * 2.5;
    let offsets = array<vec2<f32>, 8>(
        vec2(1.0, 0.0),
        vec2(-1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(0.0, -1.0),
        vec2(0.707, 0.707),
        vec2(-0.707, 0.707),
        vec2(0.707, -0.707),
        vec2(-0.707, -0.707),
    );
    var occlusion = 0.0;
    for (var i = 0u; i < 8u; i++) {
        let sample_uv = clamp(uv + offsets[i] * step_uv, vec2(0.001), vec2(0.999));
        let sd = opaque_depth_at(sample_uv);
        if !is_sky(sd) {
            let sp = view_position(sample_uv, sd);
            let diff = sp - view_pos;
            let dist = length(diff);
            if dist > 0.02 && dist < 3.5 {
                let horizon = clamp(diff.z / dist, 0.0, 1.0);
                occlusion += horizon * smoothstep(3.5, 0.2, dist);
            }
        }
    }
    return clamp(1.0 - (occlusion * 0.18 * ao_strength), 0.15, 1.0);
}

@fragment fn fs_main(in: StageIn) -> @location(0) vec4<f32> {
    let base_srgb = textureSampleLevel(scene_color, linear_sampler, in.uv, 0.0).rgb;
    var color = srgb_to_linear(base_srgb);
    let depth = opaque_depth_at(in.uv);
    let light_dir = normalize(select(frame.moon.xyz, frame.sun.xyz, frame.sun.y >= -0.05));
    let sun_elevation = clamp(frame.sun.y, -1.0, 1.0);
    let day_factor = smoothstep(-0.08, 0.18, sun_elevation);
    let sunset_factor = smoothstep(-0.12, 0.05, sun_elevation) * (1.0 - smoothstep(0.05, 0.35, sun_elevation));

    if is_sky(depth) {
        let ray = normalize(world_offset(in.uv, 1e-5));
        let sun_dot = max(dot(ray, frame.sun.xyz), 0.0);
        let mie = pow(sun_dot, 12.0) * (0.35 + 0.65 * sunset_factor) * (1.0 - frame.weather.x * 0.8);
        let sun_tint = mix(vec3(1.0, 0.88, 0.68), vec3(1.0, 0.45, 0.18), sunset_factor);
        color += sun_tint * mie * 0.85;
        return vec4(color, 1.0);
    }

    let world_rel = world_offset(in.uv, depth);
    let view_pos = view_position(in.uv, depth);
    let normal = world_normal_at(in.uv);
    let mat = material_at(in.uv);

    // Ambient occlusion from depth discontinuity neighborhood.
    let ao = compute_ao(in.uv, depth, view_pos);

    // Directional sun/moon shadow factor.
    var shadow = 1.0;
    if shadows && frame.dimension.y > 0.5 {
        let raw_shadow = sample_shadow(world_rel, normal);
        shadow = mix(1.0, raw_shadow, shadow_strength);
    }

    let ndotl = max(dot(normal, light_dir), 0.0);
    let sun_color = mix(
        vec3(0.22, 0.28, 0.48),
        mix(vec3(1.05, 0.96, 0.86), vec3(1.18, 0.58, 0.28), sunset_factor),
        day_factor,
    );
    let sky_ambient = mix(vec3(0.08, 0.11, 0.20), srgb_to_linear(frame.sky_color.rgb) * 0.55, day_factor);
    let direct_light = sun_color * ndotl * shadow * (1.0 - frame.weather.x * 0.55);
    let bounce_light = sky_ambient * (0.45 + 0.55 * max(normal.y, 0.0)) * ao;

    // Wetness specular sheen during rain on upward-facing surfaces.
    let wetness = frame_ext.environment.z;
    let view_dir = normalize(-world_rel);
    let half_vec = normalize(light_dir + view_dir);
    let wet_mask = wetness * smoothstep(0.65, 0.95, normal.y) * mat.g;
    let spec = pow(max(dot(normal, half_vec), 0.0), 48.0) * wet_mask * shadow * 0.65;

    let shade_boost = mix(0.68 * ao, 1.0 + 0.35 * ndotl * shadow, 0.65);
    color = color * shade_boost + color * direct_light * 0.32 + bounce_light * 0.08 + sun_color * spec;
    return vec4(max(color, vec3(0.0)), 1.0);
}
