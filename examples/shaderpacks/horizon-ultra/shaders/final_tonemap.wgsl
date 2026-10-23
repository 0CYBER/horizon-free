// Stage 4 (Final): Combines full-resolution scene with bloom/godrays, applies
// exposure, ACES filmic tonemapping, saturation, and cinematic vignette.
override exposure: f32 = 1.10;
override saturation: f32 = 1.12;
override tonemap_aces: bool = true;
override vignette: bool = true;

@fragment fn fs_main(in: StageIn) -> @location(0) vec4<f32> {
    var hdr = textureSampleLevel(previous, linear_sampler, in.uv, 0.0).rgb * exposure;

    if tonemap_aces {
        hdr = aces_tonemap(hdr);
    } else {
        hdr = clamp(hdr, vec3(0.0), vec3(1.0));
    }

    var srgb = linear_to_srgb(hdr);
    let luma = luminance(srgb);
    let rain_desat = 1.0 - 0.25 * max(frame.weather.x, frame.weather.y);
    srgb = mix(vec3(luma), srgb, saturation * rain_desat);

    if vignette {
        let aspect = frame.resolution.x * frame.resolution.w;
        let d = length((in.uv - 0.5) * vec2(aspect, 1.0));
        srgb *= 1.0 - 0.26 * smoothstep(0.48, 0.98, d);
    }

    return vec4(clamp(srgb, vec3(0.0), vec3(1.0)), 1.0);
}
