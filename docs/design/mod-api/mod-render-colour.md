# Mod render colour

[The adopted rule (arismoko/horizon#1498)](https://github.com/arismoko/horizon/issues/1498#issuecomment-6691333172)
is shared by mod pipelines, mod passes and shader packs:

Colours are linear light, 1.0 is vanilla's white at paper white, and values above
1 are allowed. Horizon's generated entry code converts to and from the frame,
as it does for 0.2 pack stages, so mod shader code only sees linear light. Mod
code never assumes the frame's format and leaves the frame's alpha to the
renderer. arismoko/horizon#869's typed configuration doesn't offer the colour format.

This is a Horizon contract, with no NeoForge counterpart. The upstream basis is
horizon-rust/horizon#9600 (HDR), horizon-rust/horizon#9930 (shader API 0.2) and
horizon-rust/horizon#10040 (serving 0.2).

## What exists today

The shared rule uses BT.709 primaries and vanilla's power-2.2 decoding, not the
piecewise sRGB curve. The conversion routes are specified in
`docs/design/hdr-linear-light.md:54`: material albedo, tint, lightmap and coloured
light; sky, fog and weather colours; block textures; and scene, previous,
history, opaque, reflection, shadow-colour and pack-target colour textures.
Depth, normals, light levels, emission and LabPBR channels stay data. Custom
noise textures also stay data (`docs/api/shaders.md:171`).

The existing pack generator supplies the conversion helpers in
`crates/horizon-shader-api/src/stages.rs:133`, frame-storage conversion at
`:510`, and linear post-chain entry code at `:610`. Mod pipelines (arismoko/horizon#884,
[Drawing with your own render pipeline](../../modding/src/how-to/client/render-pipelines.md)) reuse the
helpers at `:133` in their generated entry points; mod passes (arismoko/horizon#867, arismoko/horizon#869) do not
exist yet. The current
experimental serving limits are in `docs/api/shaders.md:163`: 0.2 needs
`HORIZON_HDR_FRAME=float`; linear frame blending is not yet served.

Instance inputs remain vanilla-space inputs: `ModelInstance::tint` is packed
`0xAARRGGBB`, and block and sky light are each `0..=15`
(`crates/horizon-mod-vocab/src/client/model_instance.rs:1`). The engine owns their
conversion. Do not pre-decode these inputs just because shader colours are
linear. R2 instance drawing (arismoko/horizon#808) must preserve this distinction.
`WorldQuad::color` remains a `u32` ARGB colour with 8-bit channels, including
coverage alpha (`crates/horizon-mod-vocab/src/client/world_draw.rs:40`). It cannot express
RGB above 1; HDR-range quad colours would need a later typed addition.

## Applying the rule

The planned R5 work (arismoko/horizon#884, arismoko/horizon#867, arismoko/horizon#869 and arismoko/horizon#889) must use the same generated
entry/exit boundary for mod pipelines and passes. Material, sky, weather and
scene-texture colour must arrive linear and leave linear; data textures must
not acquire colour decoding. Coexistence with a selected shader pack must not
introduce a second decode, encode or mod-specific colour convention.

For example, neutral shader RGB `(1.0, 1.0, 1.0)` means paper white. RGB
`(2.0, 2.0, 2.0)` means twice that linear light. In a gamma-stored float frame,
the generated exit stores about `1.37035` for linear `2.0` (power `1/2.2`), and
the next generated colour entry restores `2.0`. In linear storage it stays
`2.0`. White stays `1.0` in either case. Encoded vanilla `0.5` becomes about
`0.21763764` in shader code; depth `0.5` stays `0.5`. These are boundary examples,
not a promise that an SDR display or an 8-bit target can retain values above
white. Output mapping and storage limits belong to the renderer.

SDR/HDR switches, float-target selection and target/pipeline recreation must
preserve this shader contract. Resource reload must rebuild affected resources
and generated boundaries together. Mods must not cache a presumed frame format.
GUI colours and alpha compositing also remain renderer-owned: vanilla-space
GUI inputs are not automatically linear shader inputs. Coverage supplied by a
draw is distinct from permission to write the frame's alpha. A mod pass cannot
use that alpha as private storage; 0.2 post alpha is ignored. The HDR GUI
layering plan is in `docs/design/hdr-linear-light.md`, under “The HUD”.

## Review checklist

For arismoko/horizon#884, arismoko/horizon#867, arismoko/horizon#869, arismoko/horizon#889 and R2 instance drawing:

- Check material, sky, weather and sampled scene colour at both generated
  boundaries; check that data textures remain data.
- Exercise white `1.0`, bright `2.0`, vanilla tint/light and 8-bit ARGB separately.
- Check pack coexistence, resource reload and SDR/HDR/float target recreation
  without stale pipelines or duplicate conversion.
- Check translucent world draws and GUI coverage over bright content; keep
  frame alpha owned by the renderer and configuration format-free.

Open question for those implementations: exact pass ordering and reload
invalidation APIs are not settled by this colour rule. Recommendation: resolve
those in the typed R5 contracts, reuse the renderer's resource lifetime, and
keep the colour boundary invariant across every supported ordering.
