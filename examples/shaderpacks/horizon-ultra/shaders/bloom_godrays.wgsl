// Stage 3 (Composite, half-resolution): HDR bloom extraction + 9-tap tent filter
// and radial light shafts (screen-space godrays) towards the sun/moon.
override godrays: bool = true;
override bloom_strength: f32 = 0.35;

@fragment fn fs_main(in: StageIn) -> @location(0) vec4<f32> {
    let base = textureSampleLevel(previous, linear_sampler, in.uv, 0.0).rgb;
    let texel = frame.resolution.zw * 2.0;

    // 9-tap soft bloom filter around bright highlights.
    var bloom = vec3(0.0);
    if bloom_strength > 0.001 {
        let offsets = array<vec2<f32>, 9>(
            vec2(0.0, 0.0),
            vec2(1.0, 0.0),
            vec2(-1.0, 0.0),
            vec2(0.0, 1.0),
            vec2(0.0, -1.0),
            vec2(1.0, 1.0),
            vec2(-1.0, 1.0),
            vec2(1.0, -1.0),
            vec2(-1.0, -1.0),
        );
        let weights = array<f32, 9>(0.25, 0.125, 0.125, 0.125, 0.125, 0.0625, 0.0625, 0.0625, 0.0625);
        for (var i = 0u; i < 9u; i++) {
            let sample_c = textureSampleLevel(previous, linear_sampler, in.uv + offsets[i] * texel, 0.0).rgb;
            let lum = luminance(sample_c);
            let bright = max(lum - 0.72, 0.0) / max(lum, 1e-4);
            bloom += sample_c * bright * weights[i];
        }
    }

    // Radial godrays from the sun's screen projection.
    var shafts = vec3(0.0);
    if godrays && frame.camera_state.x == 0.0 && frame.dimension.y > 0.5 {
        let sun_clip = frame.projection * (frame.view * vec4(frame.sun.xyz * 128.0, 1.0));
        if sun_clip.w > 0.0 {
            let sun_ndc = sun_clip.xy / sun_clip.w;
            let sun_uv = vec2(sun_ndc.x * 0.5 + 0.5, 0.5 - sun_ndc.y * 0.5);
            let delta = (sun_uv - in.uv) / 12.0;
            var uv_Cursor = in.uv;
            var transmittance = 0.0;
            var decay = 1.0;
            for (var s = 0u; s < 12u; s++) {
                uv_Cursor += delta;
                let sample_uv = clamp(uv_Cursor, vec2(0.001), vec2(0.999));
                if is_sky(depth_at(sample_uv)) {
                    transmittance += decay;
                }
                decay *= 0.92;
            }
            let ray = normalize(world_offset(in.uv, 1e-5));
            let sun_facing = pow(max(dot(ray, frame.sun.xyz), 0.0), 4.0);
            let sunset = smoothstep(-0.1, 0.25, frame.sun.y);
            let shaft_color = mix(vec3(1.0, 0.55, 0.25), vec3(1.0, 0.92, 0.75), sunset);
            shafts = shaft_color * (transmittance / 12.0) * sun_facing * 0.28 * (1.0 - frame.weather.x);
        }
    }

    return vec4(base + bloom * bloom_strength + shafts, 1.0);
}
