# Traced Water

The smallest pack that traces rays from the material stage ([shaders.md](../../../../docs/shaders.md#ray-tracing),
[shaders-reference-0.2.md](../../../../docs/api/shaders-reference-0.2.md)): water reflects its view off its surface
with `trace` and mixes in the colour of what the ray hits, or the sky's where it hits nothing. Every other surface
is vanilla's. It shows the call, not a look: one ray a water fragment, the hit unlit, no fresnel. `trace` reads solid
terrain alone, so a ray passes through other water (`trace_layers` finds it). Where the game doesn't trace
(`frame.has_ray_tracing` is 0: a GPU without ray queries, or `HORIZON_RAY_TRACING=1` not set) the water stays
vanilla's. 0.2 is experimental, so this is a test pack of the SDK: the game draws it only with
`HORIZON_HDR_FRAME=float` set, and the registry doesn't list it. Check it with
`horizon-shader check crates/horizon-shader-api/packs/traced_water`.
