// The look's tone curve, which fits the world's light to the display: as written where the
// display reaches the scene's peak, eased into the display's peak below it. In SDR nothing was
// lifted, and Horizon clips at white.
fn tonemap(input: TonemapInput) -> vec3<f32> {
    if frame.hdr == 0u {
        return input.color;
    }
    let top = max(input.color.r, max(input.color.g, input.color.b));
    // The scene's peak times `frame.hdr` (1 here): a value the shader compiler meets at run time,
    // as Horizon's own tone curve has it. As a bare constant DX12's compiler folds it into PQ's
    // constants and rounds a few pixels a step apart.
    let peak = HDR_SCENE_PEAK * f32(frame.hdr);
    return input.color * hdr_tone_scale(top, peak, frame.headroom, frame.paper_white);
}
