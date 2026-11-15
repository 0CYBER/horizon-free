# Linear light for HDR and shader packs (shader API 0.2)

A design note, before any code: what a colour means to a shader pack from API 0.2 on, how the frame stores the
picture, and how HDR output stops reading light from the frame's alpha. Part of [#3125](https://github.com/horizon-rust/horizon/issues/3125)
and [#1441](https://github.com/horizon-rust/horizon/issues/1441). The shader API is experimental, which needs a
design note for a new line ([api/stability.md](../api/stability.md)); this is that note.

## Why

**The frame can't hold light brighter than white.** The world is drawn in vanilla's gamma space into the swapchain's
8-bit format, or with HDR on into `Rgba16Unorm`, which clips at white after every pass as the 8-bit frame does.
HDR output (`hdr.wgsl`) rebuilds the light above white at the end, from marks in the frame's
alpha:

| Alpha | Means |
| --- | --- |
| below 0.75 | a light source: its tier and how much it is lifted |
| 0.75 and above | nothing lifted |
| 1/255 | the first-person hand |
| what the HUD and menus leave | how much they cover the world |

To tell the world from the HUD, `Hdr::keep_world` copies the whole frame every frame and `shows_world` compares
the two within a tolerance. Anything that filters or blends alpha can break a mark: FXAA and TAA return the centre
pixel's alpha with smoothed colour (`world_post.wgsl` 54 and 132), and the resolution-scale copy (`fs_copy`) samples
alpha bilinearly, so a pixel between a torch and a wall can land in another tier's band. (Found in the code, not yet
measured on screen.)

**Packs can't make HDR.** A pack's last post pass draws onto that clipped frame, HDR lighting turns off while a pack's
pack runs, and post passes must return `input.color.a` so the marks survive. A pack's HDR would only
be possible by guessing highlights back from an SDR picture (inverse tone mapping).

**The API doesn't say what a colour is.** The reference gives colours as "0..1" and never says gamma or linear, or
whether more than 1 is allowed. The example pack reads them as gamma, capped at 1: Golden Hour's glow picks pixels by
`smoothstep(0.8, 1.0, luminance(c))` and its grade clamps to 0..1. Every pack written that way makes the meaning
harder to change.

**Vanilla is going the other way.** Mojang plans Vibrant Visuals for Java Edition (the render thread and the move to
Vulkan are its groundwork). On Bedrock it lights in HDR, in physical units, with auto exposure, bloom and a filmic
tone curve down to SDR at the end. A pack API capped at gamma 0..1 would have to break again for it.

## Goals

1. **HDR off and no pack: the picture is today's, bit for bit**, pinned by a screenshot test (below). Players without
   HDR or a pack run the code they run today.
2. **0.1 packs kept working until 0.2 was on by default**: unchanged in SDR, through a shim that went with them
   (below).
3. **Packs write light above white, on SDR and HDR alike.** SDR is HDR with no headroom: one pack, one tone curve,
   both displays.
4. **No light in alpha** for HDR without a pack and for 0.2 packs: the light-source and HDR-lighting marks give way
   to real values. Alpha keeps only how much the HUD covers.

Not goals: changing vanilla's look; physically based lighting without a pack; HDR for `shaders/`-format packs (see
below); where HDR's settings live (a later note).

## The colour rule (shader API 0.2)

- **Every colour a 0.2 stage reads or returns is linear light**: BT.709 (sRGB) primaries by default,
  scene-referred, **1.0 is vanilla's white at paper white**, more is brighter, up to half float's range. Horizon
  replaces negative values and NaN with 0 before the display.
- **Decoding is vanilla's 2.2 power**, as `hdr.wgsl` decodes today, not the piecewise sRGB curve. Because a power is
  multiplicative, vanilla's lit colour `albedo * tint * lightmap` is the same in linear: decoding the product is the
  product of the decoded terms.
- **What converts:** `MaterialInput` `albedo`, `tint`, `lightmap` and `colored_light`; `SkyInput.color`;
  `WeatherInput.color`; `Frame` `sky_color` and `fog_color`; `block_texture` and `block_texture_level`; every
  colour texture a stage samples (`scene_color`, `previous`, `history`, pack targets, `opaque_color`, `reflection`,
  `shadow_color`, `block_light_colors`). Data stays data: depth, normals, `light` levels, `emission`, and every
  channel of the LabPBR maps (`block_normal_map`, `block_specular_map` and their `_level` forms).
- **Alpha is coverage only.** The rule to return `input.color.a` goes; a 0.2 post pass's alpha is ignored.
- **Display info in `Frame`**, appended after `shadow_center_texel`:

| Field | Type | Meaning |
| --- | --- | --- |
| `hdr` | `u32` | 1 while the output is HDR, else 0. |
| `headroom` | `f32` | The peak over paper white: the brightest value the display shows (Apple's EDR headroom, Android's HDR/SDR ratio). 1 in SDR. It changes while the game runs (the system's SDR white, a calibration, another display), so read it every frame. |
| `paper_white` | `f32` | Paper white in nits; a nominal 80 in SDR (see the decisions at the end). |
| `peak` | `f32` | The peak brightness in nits; a nominal 80 in SDR, as `paper_white`. |
| `black_level` | `f32` | The display's black in nits (HDR's Black Level); 0 in SDR. |

- **A `[tonemap]` stage**, `fn tonemap(input: TonemapInput) -> vec3<f32>`: the post chain's linear picture in, the
  picture for the display out, linear, 1.0 at paper white and at most `frame.headroom`. It runs once per pixel at
  the window's size, after the post chain and before the camera overlays and the HUD, and owns the look of
  highlights. A pack that leaves it out gets Horizon's:
  in HDR a roll-off into the headroom (ITU-R BT.2390's EETF in PQ; HDR's black level already uses the same
  recommendation's lift), in SDR a clip at white, as vanilla. A pack that only grades keeps Horizon's tone curve;
  one that only tone maps needs no post chain. Horizon's curve keeps light as it is up to its knee, 60% of the peak
  and at least paper white. The source's peak is where the EETF
  puts its knee there (13.7 times paper white at 203 and 1000 nits, PQ's 10 000 nits at most), and light past it
  shows at the peak. The curve acts on the brightest channel and scales the others with it, so light keeps its hue
  as it eases and no channel passes the peak.
- **Horizon's display pass is always last** and can't be replaced: it clamps to the headroom, converts to BT.2020
  for HDR10, lifts the black level, encodes (PQ, scRGB, or 2.2 for SDR) and shows what the HUD and menus cover at
  UI Brightness, as today.

## How the frame stores the picture

The frame depends on what draws the picture, not on the display:

| Picture | Frame | Stored as |
| --- | --- | --- |
| HDR off, no pack | the swapchain's 8-bit format, as today | vanilla's gamma, clipped |
| HDR on, no pack | `Rgba16Float` | vanilla's gamma, values above 1 kept |
| a 0.2 pack (SDR or HDR) | `Rgba16Float` | vanilla's gamma, values above 1 kept; linear light with `[frame] blend = "linear"` |
| a `shaders/`-format pack | as today | as today, with nothing lifted in HDR (see `shaders/`-format packs) |

**When it changes.** Today the frame's format is chosen when the renderer is made and then kept: HDR turned off keeps
the 16-bit frame until the next start, and dropping a pack rebuilds nothing. A float frame can't stay behind like that,
because it doesn't clip as the 8-bit one does. Whenever the row above changes (HDR on or off, a 0.2 pack picked or
dropped), the frame and the pipelines that draw into it are rebuilt before the next
frame. What that costs is measured in the series.

**Why gamma storage.** Vanilla blends glass, water, clouds, particles and weather by fixed-function blending in
gamma space. Keeping that encoding keeps every blend of values up to 1 as vanilla's, and solid surfaces and lighting
are the same in either encoding. With HDR on and the Vanilla look the picture is SDR's except where vanilla would have
clipped at white before a later blend: the sun and moon add onto the sky (`RenderPipelines.CELESTIAL`, src alpha and
one), so clouds or glass over a bright sun come out a little brighter than vanilla's. Goal 1 covers HDR off only;
this case is held to the existing HDR-versus-SDR check (below).

**Linear blending, if a pack asks.** `[frame] blend = "linear"` in the manifest stores linear light instead, for a
physically based pack that wants glass, water and smoke to blend as light does. A 0.2 stage sees the same values
either way; what changes is where Horizon converts:

| | `blend = "gamma"` (the default) | `blend = "linear"` |
| --- | --- | --- |
| The frame holds | vanilla's gamma, values above 1 kept | linear light |
| A 0.2 stage's result | encoded before the frame stores and blends it | stored as it is |
| Horizon's own draws (entities, particles, clouds, weather, stages the pack leaves out) | as today | decoded at the end of their shaders: solid ones look as with gamma, see-through ones blend in linear |
| Horizon's multiplying draws (block breaking, `CRUMBLING_BLEND`) | as today | their factor given in linear terms, so they darken as vanilla's |
| Horizon's fog | mixed in gamma, as vanilla's | mixed in gamma too: encoded, mixed, decoded |
| What a 0.2 stage samples from the frame (`scene_color`, `opaque_color`) | decoded: once before the post chain (a full-screen pass), and in the copy `opaque_color` already is | as stored |

The water mirror and `shadow_color` draw no blends, so they store linear in both.

**Cost and support.**

- `Rgba16Float` is 8 bytes a pixel, as `Rgba16Unorm` is. HDR on keeps two targets at the frame's size, as today:
  the frame, and the HUD's target in place of `keep_world`'s copy, plus the HUD's one-byte coverage.
- A 0.2 pack in SDR moves the frame from 4 to 8 bytes a pixel (about 15 MB more at 1440p), only while it runs. Its
  post chain already uses `Rgba16Float`.
- WebGPU renders to, blends and filters `Rgba16Float` without optional features, unlike `Rgba16Unorm`
  (`draws_frame`); the 8-bit fallback stays only for the HUD's target. The GL backend is checked in the series.
- Up to 1, half float steps at least 8 times finer than 8-bit, and far finer in the darks, so Smooth Gradients'
  gain stays without its setting.

## Light above white without marks

Each shader that marks the frame today writes the light instead:

- **Light sources** (`mesher` `EMISSION_BITS`, `hdr_tier`, `hdr_look.wgsl`'s tiers): the face's colour times its tier's
  gain, in units of paper white. Easing into the peak moves to the tone curve. A placed block's texel leaves the
  light that falls on it behind as it lifts (`hdr_unshade`, terrain.wgsl `solid_finish_at`): the face's shading,
  ambient occlusion and its light level lose their darkening by the texel's lift, so a fully lifted texel shines
  the same on every face and an unlifted one stays as SDR draws it. The vertex colour is tint, shade and the
  lightmap in one, so the shader scales its brightest channel rather than dividing the shade out, and to the
  lightmap's brightest channel at full light (sky 15, block 15), not to 1: that is 1 unless something dims all
  light, the Darkness effect or a wither's bar, and a light source dims with the rest of the world. The hue stays
  the shade's on purpose: it is the hue SDR and the frame before showed for that face, and this changes brightness
  only. The scale is never below 1, since a coloured light adds past the lightmap. Sections, fluids, distant
  terrain and the visibility buffer share the one place. Items and block entities keep their shading: their
  vertex colour is their own colour, the diffuse light is baked in the vertex stage, and the entity fragment
  stage has no input left to carry the two apart (#8195); and a dropped torch lights nothing, so its lightmap
  should still darken it.
- **Lit surfaces** get no gain, however strongly the sun or a torch lights them: diffuse white stays at paper white,
  where ITU-R BT.2408 puts it, and HDR's range above it is the light sources' and the sun's.
- **Where:** terrain and fluids (`terrain.wgsl`, `mesher/fluid.rs`, distant terrain in `lod/mesh.rs`, the visibility
  buffer), particles (`particles.rs`), the sun and moon (`fs_sky_celestial`, `fs_sky_sun`), entities, dropped and held items
  (`entity.wgsl`, `dynamic_light.rs`) and lightning (`gfx/lightning.rs`).
- **The hand** draws at gain 1, as it stays at the world's brightness today, and before `keep_world` (see the HUD
  below), so it needs no mark.

The target look for HDR on without a pack is today's, at Auto (203 and 1000 nits), with two kinds of difference
allowed, shown in each pull request's screenshots:

- **Rounding**: half floats and 8-bit steps round apart. Nothing is tuned to match them.
- **Light behaving as light** wherever something is drawn over it, as the sun under clouds already does (see Why gamma
  storage): glass and smoke over a light source let its light through, instead of a blended mark lifting both; what
  is under water shines at its own light, instead of the water surface's (`fs_translucent_mark` lights the whole
  pixel today); anti-aliasing smooths the light's own edges.

Artifacts are not allowed: light showing through the HUD or a menu, light lost or lifted twice where a marked draw
meets written light, anti-aliasing or TAA history smearing or sparkling around it. Any other change of look is a
separate pull request, with screenshots.

Under a 0.2 pack Horizon lifts nothing: the pack gets vanilla's picture in linear light whatever the display, reads
`emission` in its material stage, and writes what shines. (Telling post stages which pixels are light sources is a
likely 0.2 addition, not designed here. Wide colour is one that is: a pack asks for BT.2020 primaries with
`[frame] primaries`, which the check takes and the game doesn't serve yet, #14584.)

## The HUD and menus

The HUD always draws in gamma, with vanilla's blends, onto a picture clipped to white, as today:

- **HDR off without a pack**: onto the frame, as today.
- **A 0.2 pack in SDR**: the display pass writes the tone curve's result to the swapchain in gamma, and the camera
  overlays and the HUD draw on it, as they draw after a pack's post chain today.
- **HDR on**, with or without a 0.2 pack, as below.

With HDR on:

- **Camera overlays belong to the world.** `hud_overlays::camera_overlays` (the vignette, the pumpkin and powder-snow
  overlays and the rest) and the hand draw onto the tone-mapped world before `keep_world` (today just after it),
  converted as Horizon's own draws are under `blend = "linear"`, so they darken or cover light above white too, the
  world border's red vignette included.
- **`keep_world` becomes a pass** instead of a copy. The frame keeps the world, light above white included; the pass
  writes it clipped to white, in gamma, with alpha 0, into the HUD's target. That backdrop is what the HUD draws on,
  so it starts at white at most, as today. The target is 8-bit, as the frame with HDR switched off is, so each HUD draw
  clips at white and rounds as it does there.
- **The HUD and menus keep today's blends, alpha included**, so the crosshair and `GUI_INVERT` (which invert what is
  under them), the text highlight, the menu blur and CIT glint layers (whose blends may read the destination's alpha,
  `hud/cit_glint.rs`) see what they see today; the backdrop's alpha 0 is the alpha `keep_world` leaves today.
- **Coverage has a target of its own**, one 8-bit channel at the same size, cleared to 0, which every HUD draw writes
  as a second output with a blend of its own, only with HDR on: how much of the backdrop is hidden. Draws that lay a
  layer over what is under it (vanilla's translucent GUI blend, and the depth-tested one) add their alpha over it
  (`One, OneMinusSrc` on their alpha); additive draws (the text highlight) leave it, since they hide nothing; the rest
  (the inversion, the End portal fill, CIT glint layers) count as covering what they draw, by their texel's alpha,
  the same way.
- **The display pass composites in gamma**, the space the HUD blends in. With `c` the coverage, `B` the backdrop and
  `P` the HUD's target, the HUD's own part is `H = P - (1 - c) * B`, exact for the layering blends. The output is `H`
  at UI Brightness (`H * ui^(1/2.2)`, so a fully covered pixel is today's covered pixel, scaled by UI Brightness in
  linear light) plus `(1 - c)` of the kept world in gamma, decoded for the display. At the default 100%, wherever the
  world is at or below white, that is `P`, today's picture. Behind see-through HUD the world keeps its light above
  white, weighted as vanilla weights the HUD over it, and doesn't dim with UI Brightness. Inverted pixels count as
  covered, so the crosshair's edges are exact where its texels are opaque.
- **The menu blur keeps light above white.** It blurs the composite so far, instead of the clipped picture, writes the
  result as the new kept world, resets the HUD's target to it, clipped, with alpha 0, and clears coverage to 0. The menu's own darkening then
  layers over it as any HUD does: a bright sun behind a paused game stays bright, dimmed with the rest, so pausing
  doesn't change the picture's brightness.
- `shows_world` and the hand's mark go.

## 0.1 packs

Shader API 0.1, the gamma line before this one, was served beside 0.2 until step 5b, on the path it had: its frame,
inputs and gamma values, and the marks its stages passed on in alpha. It is retired:

- a 0.1 pack is refused with the message stability.md asks for (`needs shader API 0.1; this Horizon serves 0.2`);
- the registry lists 0.2 packs instead of 0.1 ones;
- Golden Hour's 0.2 port replaced it as the example and test pack;
- the changelog says so.

The shader API is experimental: it may change in any release without notice, with no deprecation window
([api/stability.md](../api/stability.md)). Serving 0.1 for longer would have kept its shim, the pipeline variants
that write marks and the HDR pass's lift from them, only for packs that can move to 0.2.

Later lines follow the same rule while the API is experimental: a line is served until the next one is on by default.
From preview on, stability.md's deprecation windows apply.

## `shaders/`-format packs

They keep their own buffers and tone map to SDR in their last program, and their alpha is theirs. Their `final`
program writes all four channels onto the frame (`gfx/pack_post.rs` `target`), and the HDR pass reads its colour
alone: their picture, on the swapchain's 8-bit frame, is shown at paper white with nothing lifted, the hand and the
HUD with it, as #3125 suggests. HDR values they could read (`hdr`, headroom,
paper white) are a later, optional addition for pack authors.

## Horizon's look

What Horizon lifts with HDR on and no pack (light sources, the sun, the tone curve's scene peak) is a look, and
step 7 makes it one a pack could have written: its numbers and its maths in one WGSL file, drawn through shader
API 0.2's public inputs.

**Not yet a pack that runs by default.** The goal, approved in
[#16126](https://github.com/horizon-rust/horizon/issues/16126), is that the look becomes a bundled pack that runs by
default and the engine carries none of it. Today a 0.2 pack, whatever its stages, turns off what the default path
has and its own doesn't yet: the compact solid terrain layout (the world is meshed again), TAA, FXAA, upscaling and
sharpening (`world_post`), frame generation, Improved Transparency and GPU posing of entities. Its bytes can't be
the default path's either: a material stage works in linear light, so the wrapper decodes and encodes with more
2.2 powers per pixel than `hdr_light.wgsl`'s writers and rounds apart (`gfx/native_material.rs`), and it runs
before fade-in and fog, where the lift today multiplies the pixel after them. And the sun's disc, the moon, stars,
lightning, additive particles and beacon beams reach no stage. Running the pack itself by default waits until the
pack path keeps those features; #16126 lists each as its own step, and the flip is its step 16.

**One look file, two users.** `hdr_look.wgsl` holds the look's constants (the tiers' windows and gains,
`HDR_SCENE_PEAK`, `HDR_SUN_CAP`) and pure functions (brightness, the lift's window and gain, `hdr_unshade`, the sun's
peak from the headroom, the tone curve), with no engine name in it.

- Until the flip, the default pipelines compile it in: today's bytes and cost. The compiled-in copy is removed at the
  flip (step 16 of #16126); the file survives in the pack. The display reaches it as `Frame.headroom` does a
  pack (the sun's peak is the headroom clamped between `HDR_SCENE_PEAK` and `HDR_SUN_CAP`), so the uniforms that carry the
  windows and gains, and the duplicated functions in `terrain.wgsl` and `entity.wgsl`, go.
- `packs/horizon_hdr` (in `horizon-shader-api`, beside Golden Hour) is a real 0.2 pack that lists the same file as
  its shared source: a material stage, an entity stage and a `[tonemap]` with the scene's peak. It is the proof
  that the API suffices, and the example to copy from. The frame identity harness draws it and holds it to the
  default path's picture within a stated tolerance (decode and encode rounding, glint).

**What the API gains**, none of it breaking a 0.2 pack:

| Name | Meaning |
| --- | --- |
| `MaterialInput.emission_threshold: f32` | The brightness a texel must reach to be the part that shines (a torch's flame, not its handle); 0 where the whole sprite shines. |
| `EntityInput.light_source: f32` | The thing's own kind of light, on `MaterialInput.emission`'s scale: light-giving particles, glowing items and sign text, eyes. `emission` stays "drawn at full light". |
| `Frame.full_light: vec3<f32>` | Vanilla's light colour at full light this frame: white unless Darkness or a wither's bar dims all light. What `hdr_unshade` scales to. |

An `[environment]` stage, for what adds light and is no lit surface (the sun, moon, stars, clouds, lightning,
additive particles, beams), by `kind`, is step 9 of #16126 (it replaces the earlier `[glow]` idea, and settles that
decision). Until then those stay Horizon's under a pack, at paper white, and the pack's gate leaves the sky out. The
windows, gains and peaks need no route: they are the look's constants.

**Fog.** Light sources lift before fog on both paths. Today the lift multiplies the fogged pixel, with
the fog factor inside it; a stage runs before fog and can't. Lifting first takes fog out of the look, so the pack
and the default path agree. It changes how a light source looks in fog: step 11 of #16126, after the pack covers
the sky and the light sources' overlays (its steps 9 and 10), a pull request of its own, with captures.

**The lift's 8-bit steps** (`hdr_quantized_mark`) stay until step 11 of #16126, so every step until then is
byte-identical, and then go in the same pull request as the fog change, with captures: a change of rounding too.

**A pack owns the look.** A player's 0.2 pack and Horizon's lift don't compose: under a pack Horizon lifts
nothing, as above, and a pack that wants Horizon's light copies it from `packs/horizon_hdr`.

**The HDR page** gets a **Look** row in place of HDR Lighting; nothing else moves, and packs are still picked on
the Shaders page.

| Running | The row reads | Greyed |
| --- | --- | --- |
| No pack | `Look: Horizon` or `Look: Vanilla` | as the rows above it |
| A 0.2 pack | `Look: <the pack's name>` | "The shader pack lights the world. Pick it under Shaders." |
| A `shaders/` pack | `Look: <the pack's name>` | "The shader pack's picture is shown at Paper White." |

Horizon is the lift. Vanilla is HDR output with nothing lifted, the SDR picture but where vanilla would have
clipped before a later blend (Why gamma storage); it stays as the one flag the engine keeps. Saved as
`hdrLook:horizon` or `hdrLook:vanilla`. An old file's `hdrLighting:false` reads as `vanilla` and `true` as
`horizon`, and only `hdrLook` is written; an older build ignores the new key and shows Horizon's look.

## Upscaling, anti-aliasing, frame generation

They read the frame as it is stored and have no marks to keep, which fixes the alpha problems
above. Native upscaling
([#1403](https://github.com/horizon-rust/horizon/issues/1403)) should take the float frame. Horizon's TAA
blends the gamma-stored frame as it is; a tone-weighted resolve is for passes that accumulate linear light (a 0.2
pack's post chain, or the frame under linear storage), where bright pixels would otherwise sparkle. Frame generation
interpolates the float frame's light before the tone curve, and the made frame goes through the tone curve, the HUD's
layer and HDR output's last pass as the real one does; a `shaders/` pack's frame keeps it off with HDR output.
The rules such a pass keeps now are in [hdr-passes.md](../hdr-passes.md).

## Tests

1. **HDR off, no pack, bit for bit.** The frame format and pipelines for this case don't change (unit tests on
   `frame_format` and the pipelines' targets). A screenshot test pins the picture: pictures of the game hold
   Minecraft's textures and can't be committed, so it is differential. A runner with a copy of Minecraft and Mesa's
   software Vulkan builds main and the change, captures fixed scenes offscreen with `HORIZON_STILL=1` (repeatable
   frames, [#9398](https://github.com/horizon-rust/horizon/pull/9398), in review), and requires identical PNGs, also
   after turning HDR on and off, and picking and dropping a 0.2 pack.
2. **Golden Hour against main**: the same capture in SDR, and the offscreen HDR capture in HDR, identical.
3. **HDR on, the Vanilla look, against SDR**: the offscreen HDR capture ([#8947](https://github.com/horizon-rust/horizon/pull/8947)),
   within one 8-bit step outside the spots vanilla clips.
4. **HDR look against today**: the same captures within a tolerance, with the differences allowed above (Light above
   white without marks) shown, and screenshots in the pull request,
   including the vignette and the world border's red warning over bright light, the crosshair, chat over a torch,
   a blurred menu, and translucent black and coloured HUD at UI Brightness 50% and 100%. Once light is written, also
   blended draws over lit terrain (a particle, glass, water, a held torch in front of it), and a short sequence of
   frames with the camera moving and TAA on, for smearing and sparkle a still picture can't show.
5. **Cost**: GPU time and memory, HDR on, against today at 1080p and 1440p, on a desktop GPU and an integrated one.
6. **Linear blending**: a 0.2 test pack with `blend = "linear"` draws solid surfaces as with gamma, and glass over
   them as linear blending gives.

## Steps

Behind `HORIZON_HDR_FRAME=float` until step 5, each a small pull request:

1. This note.
2. Shader API 0.2 in `horizon-shader-api`: the rule, the `Frame` fields, `[tonemap]`, `[frame] blend`, both lines
   served, the reference regenerated, Golden Hour's 0.2 port.
3. The float frame with HDR on: the frame and the display pass decoding it; then the HUD's own layer, with the world
   still marked: a pass that turns the marks into light once, after the world and the hand are drawn (the lift the
   display pass does today, moved there), so that the camera overlays, the blur and the composite after it see light
   and the display pass lifts nothing; camera overlays before `keep_world`, the HUD's target and coverage, the
   display pass's composite, the blur; then the tone curve, after that pass, taking over the easing into the peak.
4. Light written instead of marked, a whole frame at a time: where a blended draw meets a surface of the other kind
   (glass, water, clouds or a particle over lit terrain, or the other way round), a pixel would keep one mark for both,
   so a frame is all written or all marked. First the rebuild when what decides the frame changes (HDR, a pack, the
   frame's switch, the HUD's layer), and TAA's history reset when the light it holds changes. Then every shader that
   marks the frame writes its light instead on the default path (one function, shared with the display pass, turns
   each mark into light), alpha 1 where opaque and coverage where blended, without the passes that only write marks,
   and with anti-aliasing and TAA taking light above white; then Improved Transparency (OIT) and the visibility
   buffer, a pull request each. Then no float frame holds marks, a 0.2 pack's included, and the pass from step 3
   goes. Without the HUD's layer, HDR without a pack keeps the compatibility frame, and its marks, until step 5.
5. On by default and the switch removed, and 0.1 no longer served (see 0.1 packs). The shim goes with it: the
   pipeline variants that write marks and the HDR pass's lift from them, in a pull request of its own.
   - 5a (done): the float frame is HDR's frame with no pack or a 0.2 one, and 0.2 packs are drawn, without the
     switch, which is gone. HDR is offered only on devices that draw the float frame and keep the HUD on a layer of
     its own: the compatibility frame no longer stands in for it there. It stays, with its marks, for HDR under a
     0.1 or a `shaders/` pack. The HDR setting's default stays Off; Auto as the default is a change of its own.
   - 5b (done): shader API 0.1 retired; the examples and the registry on 0.2. The compatibility frame stayed for
     HDR under a `shaders/` pack until 5c.
   - 5c (done): the shim, its marks and Smooth Gradients' 16-bit frame deleted. Under a `shaders/` pack HDR output
     shows the pack's 8-bit picture at paper white, the HUD with it, so UI Brightness doesn't reach it.
6. One curve from the scene to the display, its constants chosen from captures.
   - 6a, one peak (before step 5, so players see one change of look): the world's light is the same on every
     display, in multiples of paper white, and only the tone curve fits it to the display. Before, the writers'
     gains followed the display's headroom and the curve then compressed that light again from a source peak it
     derived from the same headroom. The rule: the scene's peak S sits just above the brightest light the world
     writes, and the world is mastered for a 1000-nit display at ITU-R BT.2408's 203-nit reference white, as HDR10
     content usually is: S = 1000/203 (4.93). The tiers multiply their brightest pixels by 1.6 (low), 2.4
     (incandescent), 2.8 (bright) and 4.0 (very bright), and lit surfaces stay at their SDR light, diffuse
     white at paper white (`hdr_look.wgsl`). The sun's disc shows vanilla's colour, sky plus sun
     clipped at white channel by channel, with white at S (`gfx/celestial.rs`, `fs_sky_sun`). The sky under it is
     vanilla's, at paper white, so a blend of the frame by a weight and a min at S give it exactly;
     added in the frame's gamma instead, sky and sun passed S and the hold flattened the disc in the sky's hue. The
     very bright tier's lift weight takes both from the plain add, so the texture's faint halo and the moon add onto
     the sky as before. The curve holds light above S at S, keeping its hue (where something adds onto the disc, or the moon
     onto a bright sky), then maps S onto the display's peak T with BT.2390's EETF in PQ on the brightest
     channel: the identity where T reaches S, that is where Peak Brightness is at least 1000/203 times Paper White,
     so at a paper white of 203 a display at 1000 nits or more shows the scene as written and one above 1000 shows
     it as at 1000, but for the sun's disc: where T passes S its white is min(T, `HDR_SUN_CAP`) (2000 nits at a paper
     white of 203; `hdr_look.wgsl` `hdr_sun_peak`), so Peak Brightness above S moves the disc alone, and nothing from the cap up.
     The disc's writer reads that peak, the one thing of the display a writer reads, and the curve holds light at
     it instead of at S: min(light, the disc's peak), the identity up to it. The frame carries no mark for the
     disc, so light that adds up past S (additive particles over a light source, lightning's quads over a bright
     background, the moon over a bright sky) is held at the disc's peak there too. Where T is at most S the disc's
     peak is S and every pass computes what it did. Light Horizon doesn't lift (the Vanilla look, the title, a 0.2 pack's own) has
     no known peak, so there the curve's source still follows the display, as the 0.2 rule above describes. HDR off,
     the HUD, the black lift and the compatibility frame (HDR's default until step 5, whose look then changes once)
     don't change.
   - 6b: the SDR screenshot from the same scene light. With surfaces and the sky at paper white at most, it keeps
     the light exactly up to paper white, where it is the SDR game's picture, and scales light above it (light
     sources, the sun) down to white by its brightest channel, hue kept. Its shoulder from 0.6 of paper white was
     made for a world lifted up to 3 times and now only darkened: white came out at 230 of 255. A knee just below
     white (0.9) keeps no more of a light source's shape in captures, and still darkens a noon sky.

7. Horizon's look as a look file and a 0.2 pack (see Horizon's look). Each pull request keeps HDR off
   byte-identical and, unless it says otherwise, the HDR picture too:
   - 7a: the Look row in place of HDR Lighting, and `hdrLook`.
   - 7b (done): the lift's numbers and maths in `hdr_look.wgsl`, which the renderer's own shaders start with. The
     windows' and gains' uniforms and the duplicated functions are gone, and the writers read the display's
     headroom where they read the sun's peak.
   - 7c (done): the tone curve in the look file (`hdr_tone_scale`), which `hdr.wgsl`'s pass calls. `scene_peak` and
     the sun's peak are gone from that pass's parameters: the look works the sun's peak out of the headroom, and
     the pass says only whether the look lifted the world. A `[tonemap]` shares the curve's scale, not the pass's
     bytes: the pass scales the stored colour in gamma and leaves what the curve doesn't move exactly as stored,
     where a stage takes linear light and returns it, through a decode and an encode. The curve for light of
     unknown range stays in the same function, and the scene's peak reaches it as a run-time value: DX12's compiler
     rounds a split curve, and a constant peak, a step apart in a few pixels.
   - 7d: `emission_threshold`, `light_source` and `full_light` in the API and its reference, then wired into the
     material and entity stages.
   - 7f: `packs/horizon_hdr`, and its session in the frame identity harness, held to the default path. The gate
     widens as inputs arrive: terrain and entities first, items and particles with the entity stage's later
     steps, the sky with `[environment]`.
   - 7e and 7g, in one pull request: light sources lift before fog on both paths, and the lift's 8-bit steps go.
     A change of look and of rounding, with captures. It is step 11 of #16126, after the pack steps 9 and 10.
   - 7b and 7c put the look in `hdr_look.wgsl`, compiled into the engine as preparation; that copy goes at the flip
     (step 16 of #16126), when the pack runs by default.

The HUD's layer and the tone curve come before light is written: once a shader writes light above white into the
frame, nothing at the end can take it back out from under the HUD (`shows_world` only skips lifting a mark), and
without the tone curve it would clip at the peak instead of easing into it.

Later, each its own note or pull request: values for `shaders/` packs. (The `[environment]` stage and running
`packs/horizon_hdr` by default are steps 9 and 16 of #16126, approved.) Additions to 0.2 that break no pack, so
they can wait: telling post stages which
pixels are light sources, and a full-screen peak for displays that dim large bright areas below `peak` (Windows
reports it, DXGI's `MaxFullFrameLuminance`; other systems mostly don't).

## Decisions for Ludovic

Each has a recommendation; the step it blocks says when it's needed.

| Decision | Options | Recommended | Why | Needed before |
| --- | --- | --- | --- | --- |
| Where the HDR-off screenshot test runs | **A.** The parity runners: build main and the change, capture fixed scenes with `HORIZON_STILL=1` on Mesa's software Vulkan, compare PNGs. **B.** A synthetic scene with generated textures and a committed digest. | A | It tests the real game with real resource packs; B's digest changes with every Mesa update and covers less. A costs a second build a run, and needs #9398. | Step 3, the first change to the frame |
| How tone mapping is written | **A.** A `[tonemap]` stage of its own. **B.** The last post pass, with a flag. | A | A pack that only grades keeps Horizon's curve, and one that only tone maps needs no post chain; the check can hold the stage to its range (0 to `headroom`); upscaling and anti-aliasing can run between the post chain and the curve. | Step 2, the API |
| `paper_white` and `peak` in SDR | **A.** 80 nits, sRGB's reference white (scRGB's 1.0). **B.** 0, for unknown. | A | A pack that divides by them never gets infinity or NaN; `hdr` = 0 already says they're nominal, and `headroom` is 1 either way. | Step 2, the API |
