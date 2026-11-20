# Traced Lights

The smallest pack that reads the scene's light sources ([shaders.md](../../../../docs/shaders.md#light-sources),
[shaders-reference-0.2.md](../../../../docs/api/shaders-reference-0.2.md)), which `lights = true` under
`[ray_tracing]` turns on: one post pass that finds each pixel's point with `view_position` and its face from the
neighbouring points, asks `ray_lights_near` which sources reach it, and for each of the first eight, read with
`ray_light`, tests with `trace_any` whether terrain stands between them. The ray stops just short of the cube of
the source's block (`block_entry`): a full glowing block such as glowstone is in the scene itself and would
otherwise stop the ray aimed at its centre. Lit sources add their colour (white unless the pack's `[[lights]]` give
one), fading out over their light level. It shows the calls, not a look: the picture's own block light stays, edges
against the sky take wrong normals, lava isn't listed, and there is no bounce or denoising. Where the game doesn't
trace (`frame.has_ray_tracing` is 0) the picture stays as it is. 0.2 is experimental, so this is a test pack of the
SDK: the game draws it, and the registry doesn't list it yet. Check it with
`horizon-shader check crates/horizon-shader-api/packs/traced_lights`.
