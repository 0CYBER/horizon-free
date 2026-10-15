// Golden hour: a warm grade, haze that glows towards the sun with distance,
// greyer when it rains, and a soft vignette. The prelude (bindings, `frame`,
// `StageIn`, depth helpers) comes first; see docs/shader-packs.md.
override saturation: f32 = 1.15;
override warmth: f32 = 0.35;
override haze: bool = true;
override vignette: bool = true;

@fragment fn fs_main(in: StageIn) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(previous, linear_sampler, in.uv, 0.0).rgb;
    let depth = depth_at(in.uv);
    let rain = max(frame.weather.x, frame.weather.y);

    // Saturation, less of it in rain.
    let grey = vec3(luminance(c));
    c = mix(grey, c, saturation * (1.0 - 0.5 * rain));

    // Warm the highlights more than the shadows.
    let warm = vec3(1.0 + 0.12 * warmth, 1.0 + 0.03 * warmth, 1.0 - 0.10 * warmth);
    c = mix(c, c * warm, smoothstep(0.1, 0.8, luminance(c)));

    if haze && frame.camera_state.x == 0.0 {
        // How much the view ray faces the sun, and how far it goes.
        let direction = normalize(world_offset(in.uv, max(depth, 1e-6)));
        let facing = max(dot(direction, frame.sun.xyz), 0.0);
        let low_sun = 1.0 - abs(frame.sun.y);
        let glow = pow(facing, 6.0) * low_sun * (1.0 - rain);
        var amount = 0.0;
        if is_sky(depth) {
            amount = glow * 0.35;
        } else {
            let distance = linear_depth(depth) / max(frame.view_params.z, 16.0);
            amount = smoothstep(0.15, 1.0, distance) * (0.15 + 0.45 * glow);
        }
        let tint = mix(frame.fog_color.rgb, vec3(1.0, 0.72, 0.42), glow);
        c = mix(c, tint, clamp(amount, 0.0, 1.0));
    }

    if vignette {
        let d = length((in.uv - 0.5) * vec2(frame.resolution.x * frame.resolution.w, 1.0));
        c *= 1.0 - 0.3 * smoothstep(0.45, 0.95, d);
    }
    return vec4(clamp(c, vec3(0.0), vec3(1.0)), 1.0);
}
