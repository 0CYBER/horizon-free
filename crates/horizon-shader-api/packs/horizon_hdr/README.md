# Horizon HDR

Horizon's own HDR look as a shader pack on the public API ([shaders.md](../../../../docs/shaders.md),
[shaders-reference-0.2.md](../../../../docs/api/shaders-reference-0.2.md)): light sources shine past paper white, each
by its kind of light, and a tone curve fits that light to the display. It is the look the game draws itself with HDR
on and no pack (Look: Horizon), and the example to copy from: a pack that wants that look under its own starts here.
Like Golden Hour it comes with the SDK, not with the game: the game never picks it by itself, and nothing changes
until a player puts the folder in `shaderpacks/` and picks it. With HDR off it lifts nothing; what else still differs
there is listed below.

`hdr_look.wgsl` is the look: its numbers (the brightness a texel lifts at, each tier's gain, the scene's peak) and
its maths, with no name of the renderer in it. There is one copy of it. The renderer's own shaders start with this
very file (`horizon-client-render`'s `gfx/wgsl_prefix.rs`) and the pack lists it as its shared source, so a change to
the look changes both. The stages only call it:

- `material.wgsl`, blocks: vanilla's colour, `albedo` under `lit_tint`, the vertex colour lit at each corner and
  smoothed over the face as Horizon smooths it. On a light source (`light_source_tier`) the texels as bright as
  `light_source_threshold`, or all of them where `light_source_whole`, take their tier's mark and, as they lift, leave
  their face's shading behind, down to `frame.full_lightmap`. It reads `lit_tint`, never the terrain's separate looks,
  so the world doesn't load again when the pack starts.
- `entity.wgsl`, entities and block entities: vanilla's colour, and what `light_source_tier` names lifted by its
  tier.
- `tonemap.wgsl`: the look's tone curve, from the scene's peak to the display's.
- `lift.wgsl`: what the first two share. The look works on colours in vanilla's gamma, a stage on linear light.

Check it with `horizon-shader check crates/horizon-shader-api/packs/horizon_hdr`. Horizon's render tests hold its
material and entity stages to the game's own shaders (`horizon-client-render`, `gfx/look_pack_tests.rs`), within a
fortieth of a step of 255: an ambient-occluded corner beside a torch, where the light level and the vertex colour
both change across a face, among them.

## What it can't draw yet

With HDR off nothing is lifted, but the points below that aren't about the lift (overlays, distant terrain) still
differ in SDR.

- **The sun's disc, the moon, stars, lightning, additive particles and beams** reach no stage. Under a pack Horizon
  draws them at paper white.
- **Items and particles** aren't drawn through `[entity]` yet, so what shines among them (a torch's flame, a glowing
  item) isn't lifted.
- **Light sources in fog, and in a section that is fading in.** Horizon fades a section toward the fog colour, then
  lifts the faded colour by a mark that only fog lowers. A stage lifts first; Horizon then fades and fogs its lifted
  light. On a fully visible surface outside fog there is no difference; in coloured fog that starts at the camera
  (under water), Horizon lifts the fog's colour with the pixel and a stage doesn't.
- **Distant terrain where it meets near terrain.** Horizon dithers tiles and sections in and out and offsets their depth
  so the right one shows where they overlap; a material stage's terrain does neither, so overlaps can show.
- **The enchantment glint on something that shines** is added after a stage, and so after the lift; Horizon lifts
  it with the pixel.
- **The back faces of a model that shines and is drawn from both sides** (an End crystal's glass). Horizon lifts
  them by their front's brightness; a stage is handed the face it draws.
- **Resource packs' emissive overlays** (OptiFine's `_e` textures), which Horizon lays over a block, in SDR too, and
  lifts: a material stage isn't handed them.
- **Screenshots and world icons in HDR.** They are taken from the frame after `[tonemap]`, so they carry its curve,
  and light above white clips channel by channel under a pack, where Horizon's look keeps its hue.
- **What any running pack turns off**: anti-aliasing (TAA, FXAA, TAA upscaling), Improved Transparency and frame
  generation.
