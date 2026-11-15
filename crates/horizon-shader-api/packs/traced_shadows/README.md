# Traced Shadows

The smallest pack that traces rays ([shaders.md](../../../../docs/shaders.md#ray-tracing),
[shaders-reference-0.2.md](../../../../docs/api/shaders-reference-0.2.md)): one post pass that finds each pixel's
point with `view_position`, asks `trace_any` whether terrain stands between it and the sun, and halves its light
where it does. It shows the call, not a look: one ray a pixel, hard edges, no light of its own. Where the game
doesn't trace (`frame.has_ray_tracing` is 0: a GPU without ray queries, or `HORIZON_RAY_TRACING=1` not set), past
`frame.ray_distance` and outside the Overworld (`frame.dimension`: the other dimensions have no sun) the picture
stays as it is. 0.2 is experimental, so this is a test pack of the SDK: the game
draws it only with `HORIZON_HDR_FRAME=float` set, and the registry doesn't list it. Check it with
`horizon-shader check crates/horizon-shader-api/packs/traced_shadows`.
