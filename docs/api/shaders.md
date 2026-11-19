# Shader API

Horizon has its own shader packs: WGSL written against stages of Horizon's renderer, checked before it runs. This
page is the **Horizon shader API**. A separate clean-room loader reads the existing `shaders/` pack format (its own
spec, not part of this API); both sit on the same renderer hooks. The earlier implementation was removed on
2026-10-24; this API was written from scratch after it.

## Overview: what a shader author will be able to do

Write WGSL against documented stages of Horizon's renderer, with typed inputs, options the player can change, a
reload as soon as a file is saved and an overlay that shows what each pass costs and draws. The renderer keeps
everything that makes it fast (culling, batching, LOD, the mesher); a pack changes how surfaces and light look, not
how the world is fed to the GPU.

## The design

- **A pack** is a folder or a zip with `horizon-shaders.toml` (id, name, version, the `api` line, the stages it
  uses, its options) and WGSL files. No preprocessor and no other shading language: WGSL is what wgpu validates and
  compiles for Vulkan, Metal and DirectX 12.
- **Stages, not pipelines.** A stage is a function with a fixed signature that the renderer calls inside its own pass,
  for example a terrain material (`fn material(input: TerrainInput) -> Surface`), the lighting of the visible
  surfaces, the sky, an optional shadow pass, and a chain of full-screen post passes ending in the frame. A pack
  that leaves a stage out gets vanilla's look for it.
- **Typed inputs.** Each stage receives structs (the frame: time, camera, sun and moon, weather, fog; the surface:
  position, normal, block and its light) whose WGSL is generated from the Rust types the renderer uploads. The
  generated reference lists every field with its unit and range, and a test fails when it drifts.
- **Options are WGSL `override` constants.** The manifest declares each option (on/off, a number in a range, a choice)
  with its default; the game shows them on the pack's settings screen and sets them when it builds the pipeline. No
  text substitution, and every option combination is validated the same way.
- **Hot reload.** The game watches the pack's folder. A saved file is validated with naga and the stage rebuilt; on an
  error the last good version keeps drawing and the error, with its file and line, is shown in the overlay.
- **Debug overlay.** GPU time per stage and pass (timestamp queries), the targets each pass writes (show any of them
  full-screen), the pack's options and its compile errors.
- **Budgets.** A pack declares the targets and passes it needs; the renderer refuses more than its limits, and the
  benchmark ([benchmark-report.md](../benchmark-report.md)) records the pack and its cost, so "fast" stays measured.
- **Off by default.** Without a pack the renderer is vanilla's look, and no pack code runs.

## The pack format (schema 1)

A pack is a folder, or the same files zipped as a `.hshader`, with `horizon-shaders.toml` at its root
(crate `horizon-shader-api`; the sample with every key is
`crates/horizon-shader-api/tests/samples/schema-1/`):

```toml
schema = 1                 # the manifest's format; a reader refuses schemas newer than it knows
id = "soft_light"          # [a-z0-9_]{1,64}: the settings key and the registry id
name = "Soft Light"
version = "1.0.0"
api = "0.2"                # the shader API line the WGSL is written for
description = "..."        # optional, like authors and license
shared = ["common.wgsl"]   # optional: WGSL put before every stage's file, in order

[sky]                      # each stage is optional, but a pack has at least one
file = "sky.wgsl"
clouds = true              # optional: false when the pack draws its own clouds
sun = true                 # optional: false when the pack draws its own sun
moon = true                # optional: false when the pack draws its own moon
stars = true               # optional: false when the pack draws its own stars

[material]
file = "material.wgsl"
files = ["pbr.wgsl"]       # optional, on any stage: WGSL put after `shared`, before this stage's file

[vertex]                   # needs [material]: where corners of terrain are drawn, for the world and the shadow map; shares the material's module
file = "vertex.wgsl"

[frame]                    # optional
fog = "pack"               # optional: the pack fogs what its stages draw itself (Horizon does under "engine", the default)

[weather]                  # the colour of rain and snow streaks
file = "weather.wgsl"

[shadow]                   # optional, needs [material] or [[post]] (0.2: or [entity]): a shadow map from the sun (the moon by night)
resolution = 2048          # its side in texels, 256 to 8192
distance = 128.0           # half the side of the square it covers, in blocks, 32 to 512
solid_materials = [10]     # optional: [[materials]] ids drawn into the map whole, cutouts ignored
option = "shadows"         # optional: drawn while this option is not 0 (else always); off, there is no shadow pass
resolution_option = "shadow_resolution"   # optional: an int, or a choice of numbers, whose value replaces resolution
distance_option = "shadow_distance"       # optional: the same for distance

[[post]]                   # full-screen passes, in the order they run (12 at most)
file = "post/grade.wgsl"
name = "Colour grade"      # optional: the GPU time overlay's label
output = "bloom"           # optional, not on the last pass: writes this target, read by name in later passes
scale = 0.5                # optional, with output: 1, 0.5 or 0.25 of the world's size

[[options]]                # 64 at most; `id` is the WGSL `override` it sets
id = "saturation"
name = "Saturation"
type = "float"             # bool, float (min, max, step), int (min, max), choice (values)
default = 1.1
min = 0.0
max = 2.0
step = 0.05

[[materials]]              # 1024 at most: MaterialInput.material (and RayHit.material), the first entry that matches
id = 10                    # 1 to 65535; 0 is a sprite no entry names
sprites = ["block/*_leaves"]   # sprite names, any number of * each
blocks = ["oak_log", "grass_block:snowy=true"]   # optional: the sprites these blocks' models draw with, where no entry's sprites name them
translucent = false        # optional: this id's sprites draw in the translucent pass, clear texels and all

[[pages]]                  # optional: the options page's layout, the first the one it opens on
id = "main"
name = "Sample"
items = ["page:colour", ""]   # a button opening a page, an empty slot (or an option's id)

[[pages]]
id = "colour"
name = "Colour"            # the page's title and its button's label
description = "How colourful the picture is."   # optional: the button's tooltip
columns = 1                # optional: 1 or 2 (default) buttons a row
items = ["saturation"]

[[textures]]               # 8 at most: a PNG every stage reads as noise and noise_sampler
id = "noise"
file = "noise.png"
filter = "linear"          # optional: linear (default) or nearest
wrap = "repeat"            # optional: repeat (default) or clamp

[[lights]]                 # 256 at most: block_light_color's colour, the first entry that matches
color = [0.3, 0.7, 1.0]    # each channel 0 to 4
blocks = ["soul_*"]        # block names, any number of * each

[[particle_lights]]        # 32 at most: these particles join the frame's coloured lights
particles = ["firefly"]    # names, `firefly` or `minecraft:firefly`; the first entry naming one wins
color = [0.8, 0.9, 0.4]    # each channel 0 to 1; the particle's alpha is its intensity
reach = 4.0                # blocks, 0.25 to 16; the 16 nearest lights shine, read by MaterialInput.colored_light

[[tags]]                   # 32 at most: bit i of MaterialInput.tags (and RayHit.tags) is the i-th tag
id = "glass"
sprites = ["block/glass", "block/*_stained_glass"]   # sprite names, at most one * each

[budget]
gpu_ms = 1.5               # optional: the GPU time the pack aims to add (0 to 16 ms)

[reflection]               # optional, needs [material]: when the water mirror (`reflection`) is drawn
option = "reflections"     # optional: drawn while this option is not 0 (else whenever the material reads it)

[displacement]             # optional, needs [material]: relief as geometry near the camera
distance = 12.0            # optional: how far, in blocks (4 to 32); eases out over the last quarter
cells = 16                 # optional: grid cells along a face's side (4 to 64)
option = "displacement_on" # optional: a bool option that turns it on (else always on)
cells_option = "cells"     # optional: a choice option of cell counts that replaces `cells`
steps_option = "steps"     # optional: a bool option: each cell a flat step with walls while it's on
```

Under `[displacement]` the faces of solid terrain whose textures have LabPBR heights (the normal map's alpha) are
cut into a grid near the camera, and each corner moves along the face's normal by the material stage's
`fn displacement(input: DisplacementInput) -> f32`: relief that is geometry, which hides itself, writes its
depth and is lit where it is. `MaterialInput.displaced` says how much of the relief a point already got that way,
so a parallax effect fades out where the geometry took over.

An option's `override` has the type of its kind: `bool`, `f32`, `i32`, or `u32` for a choice (the index of the
value). The pack holds only the manifest, the `.wgsl` files it names (each used, UTF-8, 256 KiB at most),
`README.md`, `LICENSE` (`.md`, `.txt`) and `icon.png`, under plain paths, 8 MiB in all. A `.hshader` is built
the same way every time (files in path order, fixed dates), so a pack's SHA-256 depends only on its files.

Native selections are saved as `horizon:<id>`; option values live in
`shaderpacks/.horizon-options/<id>.txt`. Updates and file renames keep both. Existing filename
selections and sidecars are imported without deleting the originals; after an update has removed
the old file, migration recognizes the launcher's `<id>-<version>.hshader` names. An arbitrary
removed filename cannot be mapped back to an ID. If several installed packs share an ID, the first
filename in lexical order is used and listed. The `HORIZON_SHADER_PACK` development override still
names an exact filesystem path; `shaders/` packs still use filenames.

<a id="shader-api-02-experimental"></a>

## Shader API 0.2

0.2 is the line this build serves, the line for linear light and HDR
([hdr-linear-light.md](../design/hdr-linear-light.md)): `horizon-shader check` and `pack` take it, the game draws it
and the registry lists it. A pack for another line is refused (`needs shader API 0.1; this Horizon serves 0.2`).
The launcher goes by the game it installed, not by its own build: it asks that game for its lines
(`--launch-api`, `content.shader.api`) and offers and installs a registry pack made for one of them.
A pack draws the world
into a float frame where the device can (else the 8-bit one): the frame follows the picked pack,
rebuilding the renderer and remeshing the world when needed. A refused or missing pack, or one whose owner mod is off,
uses the no-pack frame; a running pack whose new version fails in a stage keeps its frame, and its last good version
keeps drawing. What a pack works with:

- **Linear light.** Every colour a stage reads or returns is linear light, 1.0 at paper white and more brighter. Horizon
  decodes vanilla's values with a 2.2 power and encodes the results the frame stores. Depth, normals, light levels,
  `emission`, the LabPBR maps and `[[textures]]` stay data. `[[lights]]` and `[[particle_lights]]` colours are still
  written in vanilla's gamma, and Horizon decodes them. Alpha is coverage only, and a post pass's alpha is ignored.
- **The display in `Frame`:** `hdr`, `headroom`, `paper_white`, `peak` and `black_level`, after `shadow_center_texel`.
  With HDR on, where the device keeps the HUD on a layer of its own, they are the display's, and the pack's picture is
  shown in HDR, with the HUD laid over it as without a pack. Otherwise they are SDR's (`hdr` 0, `headroom` 1,
  `paper_white` and `peak` 80, `black_level` 0), and the picture ends on an 8-bit frame before the camera overlays
  and the HUD. Horizon lifts the shadows onto `black_level` after `[tonemap]`.
- **`[tonemap]`**, `fn tonemap(input: TonemapInput) -> vec3<f32>`: the post chain's picture in, the picture for the
  display out, from 0 to `frame.headroom`, once per pixel of the window. Without it a pack gets Horizon's curve, the
  one HDR uses without a pack; one or the other runs, never both.
- **`[frame] blend`**: `"gamma"` (the default) or `"linear"`, what the frame stores and so how see-through draws
  blend. Under `"linear"` the frame holds linear light: the stages' results are stored as they are and Horizon's own
  draws are decoded, so glass, water and smoke blend as light does and solid surfaces look as under `"gamma"`. A stage
  sees the same values either way. Where the device can't draw a float frame the pack draws on an 8-bit one, in
  gamma. Resource packs' `core/sky`, `core/stars` and `core/rendertype_lines` shaders write gamma, so under
  `"linear"` Horizon's own sky, stars and block outline draw in their place.
- **`[entity]`**, `fn entity(input: EntityInput) -> Surface`: the lit colour of what is no block terrain (entities,
  block entities, dropped and framed items, particles; `input.kind` says which), from its texture, tint, vanilla's
  shade and hurt flash, its light and a normal, with the material stage's shadow maps and coloured lights. It returns
  the material stage's `Surface`, so a pack lights both with one function. The game draws entities and block
  entities through it so far (`kind` 0 and 1); items and particles keep Horizon's lighting until later releases, and
  the reference lists what stays Horizon's.
- **`[frame] primaries`**: `"bt709"` (the default) or `"bt2020"`, the primaries of every colour a stage reads or
  returns. BT.2020 holds colours outside sRGB, which HDR displays show. It needs `blend = "linear"`. `Frame` says
  which in `primaries`, with that space's luminance weights in `luminance`, after `black_level`. `"bt2020"` passes
  the check, but the game doesn't serve it yet.
- **`[motion]`** (no keys, needs `[[post]]`): every post pass gets `fn motion_at(texel: vec2<i32>) -> MotionData`, how
  each pixel's surface moved since last frame, for temporal passes. The check takes it, but **the game doesn't draw
  it yet** and refuses a pack that has it.
- **`[material] coverage`**: `"engine"` (the default) or `"pack"`, whose alpha decides which points of terrain are
  drawn. Under `"engine"` solid and cutout terrain keep Horizon's alpha test on the texture's alpha and ignore the
  returned alpha, translucent terrain is cut on the texture's alpha too and takes the returned alpha as its opacity,
  and the check refuses a `discard` reachable from `material`. Under `"pack"` the returned alpha drives the alpha
  test and the stage may `discard`.
- **`[entity] coverage`**: the same for the entity stage. Under `"engine"` (the default) an entity keeps Horizon's
  coverage: the texture's alpha times the layer's opacity is cut below 0.1 and blends, the returned alpha is ignored,
  and the check refuses a `discard` reachable from `entity`. Under `"pack"` the returned alpha is cut and blends, and
  the stage may `discard`. Dissolve masks and culling are Horizon's under either.
- **`[frame] fog`**: `"engine"` (the default) or `"pack"`, which fogs what its stages draw itself. Under `"pack"`
  Horizon skips its fog on what `material` and `entity` return, and every stage gets `vanilla_fog(position)`,
  vanilla's fog at a point, with the frame's two fog distances it reads (`fog_environment`, `fog_render`), so a pack
  can start from vanilla's fog and change it. What Horizon draws itself (items, particles, clouds, the hand) keeps
  vanilla's fog.

```toml
[tonemap]                  # optional: the pack's own tone curve
file = "tonemap.wgsl"

[material]
file = "material.wgsl"
coverage = "engine"        # optional: engine (default) or pack, when the material cuts its own holes

[entity]                   # optional: the colour of entities and block entities (items and particles later)
file = "entity.wgsl"
coverage = "engine"        # optional: engine (default) or pack, when the stage cuts or fades entities itself

[frame]
blend = "gamma"            # optional: gamma (default) or linear
primaries = "bt709"        # optional: bt709 (default) or bt2020, which needs blend = "linear"
fog = "engine"             # optional: engine (default) or pack, when the pack fogs what its stages draw

[ray_tracing]              # optional, needs [[post]] or [material], no keys: the post stage gets `trace` and `trace_any`, the material stage where it calls them
[motion]                   # optional, needs [[post]]: the post stage's motion guide (not drawn yet)
```

API 0.1, the gamma line before it, is retired. To move a pack over, set `api = "0.2"` and treat every colour as
linear light: decode a constant written for display values with a 2.2 power, compute luminance with
`frame.luminance`, and stop returning `input.color.a` from post passes (their alpha is ignored). The example pack,
`crates/horizon-shader-api/packs/golden_hour`, was moved this way. `packs/traced_shadows` and
`packs/traced_water` are the smallest packs that trace rays, from a post pass and from the material stage
([the guide](../shaders.md#ray-tracing)). `packs/horizon_hdr` is Horizon's own HDR look as a pack, from the look file
the renderer's own shaders start with: it reads the light-source fields, `frame.full_lightmap` and `lit_tint`
(changelog, 2026-11-19), and its README lists what a pack can't draw of the look yet.

A 0.2 pack from before 2026-11-16 moves to the terrain's own inputs so: where it drew vanilla's colour with
`input.tint` or `hit.tint`, it reads `shaded_tint` (the old value, the shade and ambient occlusion in it), or lights
with `tint`, `shade` and `ao` apart; `[material] fog = false` becomes `[frame] fog = "pack"`; `frame.fog_start` and
`fog_end` become `fog_environment` and `fog_render`; the post stage's `history` becomes a
`[[resource]]` with `keep = "view"` that a pass writes as its `output` and reads as `<name>_previous` next frame; and
`EntityInput.emission` is `full_light`. A terrain hit's `emission` is the block's light level over 15 from
2026-11-18, as `input.emission`, no longer its brightness class. `shaded_tint` and `MaterialInput.emission` take no
look storage, and reading them triggers no remesh (a pack's LabPBR maps still can, the first time one starts); reading `tint`, `shade`, `ao` or `face` makes the world
load again when the pack starts and terrain take a third again its memory.

## Testing a pack frame by frame

`horizon-ctl screenshot-sequence FRAMES STEP_MS PATH`, under `HORIZON_STILL`, starts the running pack over and draws
its first FRAMES frames on a set clock: frame n has `frame_index` n, `time` (n - 1) × STEP_MS / 1000 and `delta_time`
STEP_MS / 1000 (0 on frame 1; STEP_MS may be a decimal, `16.666667` for 60 fps), and the eased values start as in
a pack just loaded. The last frame is written as a PNG, the same bytes every run on the same build, GPU
and driver. Packs in the `shaders/` format get the same (`frameCounter`, `frameTimeCounter`, `frameTime`). How to use
it: [the guide](../shaders.md#testing-a-pack-frame-by-frame).

## Guide, reference and examples

- [shaders-reference-0.2.md](shaders-reference-0.2.md): every stage's signature, input and output structs, resources
  and helpers, the frame's fields with their units, option types and limits, generated from
  `crates/horizon-shader-api`.
- [The guide](../shaders.md): making a pack with `horizon-shader new`, `check` and `pack`; the templates and the
  example pack (`crates/horizon-shader-api/packs/golden_hour`).

## Stability

API 0.2 is final, at the preview level of [stability.md](stability.md): #17834 was its last breaking change. What
the reference says stays: additions and fixes toward it go on in 0.2, and a change that breaks a 0.2 pack only comes
with a new line, 0.3, after a deprecation while the game still serves 0.2. API 0.1 has ended: it is refused since #15886,
when 0.2 went on by default ([hdr-linear-light.md](../design/hdr-linear-light.md), step 5).

## Not there yet

An overlay showing what each pass draws (F3 already shows what each costs), and items and particles drawn through a
pack: shader API 0.2's `[entity]` stage has their kinds, and the game draws entities and block entities through it,
but still lights dropped and framed items and particles itself. Block entities kept on the GPU (chests, banners) cast
no shadow into a pack's shadow map yet, and the first-person hand has no stage.

## Changelog

- 2026-11-19: API 0.2 (addition, preview): a pack without `[[post]]` whose frame holds gamma light (not
  `blend = "linear"` on the float frame) gets the player's FXAA, at the world's size, before its `[tonemap]` (#16126
  step 6). TAA and upscaling stay off under every pack, and a pack with a post chain is unchanged. With FXAA off
  nothing changes.
- 2026-11-19: API 0.2 (addition, preview): `MaterialInput.lit_tint`, vanilla's lit vertex colour as Horizon's terrain
  draws it: `shaded_tint` times the lightmap (coloured lights added under it) at each corner, then smoothed over the
  face, so `albedo * lit_tint` is vanilla's lit colour where multiplying the two smoothed apart is not (6.6 steps of
  255 off at an occluded corner beside a torch). Decoded as `shaded_tint` is, declared after the light-source fields
  and before `shaded_tint` only for a pack whose files name it. It takes one varying, the terrain's last on the weakest
  GPUs, and no look storage; a pack that doesn't name it sees the same prelude and entry points as before, and its
  picture is unchanged. `packs/horizon_hdr` draws its blocks with it.
- 2026-11-19: API 0.2 (additions, preview): what Horizon's own HDR look reads of light sources, for a pack that
  draws it. `MaterialInput.light_source_tier` (0 none, 1 low to 4 very bright), `light_source_whole` (the whole sprite
  shines) and `light_source_threshold` (the brightness a texel of the sprite must reach to shine), and
  `EntityInput.light_source_tier`, declared after their other fields (before `shaded_tint`) only for a pack whose
  files name one of the three; and `frame.full_lightmap`, the lightmap at full block and sky light, declared only for
  a pack whose files name it, at the end of `Frame`. The vertex, the sprite and the lightmap already hold them: they
  cost no vertex data, no look storage and no reload. A pack that names none of them sees the same prelude and entry
  points as before, and its picture is unchanged.
- 2026-11-18: API 0.2 (terrain ray-hit fix): a terrain hit's `tint`, `shade` and `ao` match
  `MaterialInput` when a usable look record is available. Reading any of them queues fresh meshes for
  loaded sections and costs eight bytes per face in CPU memory for loaded sections and in video
  memory within the ray scene's reach. Without a usable record, `tint` uses the baked vertex
  colour, with `shade` and `ao` both 1. `shaded_tint` and `emission` come from the vertex.
- 2026-11-18: API 0.2 (preview), a fix toward the reference: a terrain hit's `RayHit.emission` is its block's light
  level over 15, as `MaterialInput.emission` (one candle 3/15, a lantern 1), no longer its brightness class (7/15,
  14/15 or 1); an entity hit's stays 0. `MaterialInput.emission` comes with the vertex, as `shaded_tint` does: of
  the terrain's inputs only `tint`, `shade`, `ao` and `face` turn on terrain look storage, which can make the world
  load again when a pack starts and needs a vertex stage that reads storage buffers. A pack that reads `emission` and
  none of them pays none of that, and terrain takes a third again its memory for them, not half. No WGSL name
  changes.
- 2026-11-18: API 0.2 is final (preview): #17834 was the last change that breaks a 0.2 pack. A breaking change now
  starts 0.3, after a deprecation. No WGSL or manifest change.
- 2026-11-17: API 0.2 (experimental, #18023): an R-relative kept texture takes its two versions at their size of the
  post targets' budget, `2 × scale² × bytes a texel / 8` render-size `rgba16float` textures, instead of 2 at any
  format and scale: `rgba16float` at scale 0.5 takes 0.5, `r32uint` at scale 1 takes 1. Packs that keep small
  textures fit where they were refused. A `rgba32float` or `rgba32uint` one at scale 1 takes 4 where it took 2, so a
  pack whose post targets took more than 4 with one of those, or two of them, is now refused; no WGSL name changes.
- 2026-11-16: API 0.2 (**breaking**, experimental): the terrain's own inputs. `MaterialInput` and `VertexInput` have
  the quad's `tint` without its shade and ambient occlusion, the face's `shade`, the corner's `ao` and its `face`;
  `MaterialInput.emission` is the block state's light level over 15 (one candle 3/15, four 12/15); `VertexInput.fluid`
  is 2 for lava. A pack whose material or vertex stage reads `tint`, `shade`, `ao`, `face` or `emission` makes the
  world load again when it starts, and terrain takes half again its memory; a device whose vertex stage reads no
  storage buffers refuses it. A pack that names `shaded_tint` gets the old `tint` last in `MaterialInput`,
  `VertexInput` and `RayHit`, free (on an entity hit, its `tint` times its `shade`). `RayHit` gains `shade`, `ao` and
  `back` and is ordered as the reference lists it; an entity hit has vanilla's entity shade, the Nether's in the
  Nether. A terrain hit's `tint`, `shade` and `ao` change value, not type, in a later version (to what
  `MaterialInput` gives), without a source break; its `shaded_tint` stays their product. `EntityInput.emission` is
  `full_light: bool`. Gone: the post stage's `history`, `[material] fog` (a check error naming `[frame] fog`), and
  `frame.fog_start` and `fog_end`; the fog's fields are declared for every pack. [Moving a
  pack](#shader-api-02-experimental) says what to read instead. A pack that reads none of these is built and draws as
  before.
- 2026-11-16: API 0.2 (additions, experimental): `[sky] moon = false` leaves out Minecraft's moon, as `sun = false`
  its sun, for a sky that draws its own; and `frame.moon_phase`, the phase of the moon Horizon draws (0 full to 7
  waxing gibbous), declared only for a pack whose files name `moon_phase`, at the end of `Frame`. A pack that sets
  neither is built from the same WGSL and drawn as before.
- 2026-11-16: API 0.2 (additions, experimental): `[entity] reads` and `[sky] reads` (#16011 item 4): the entity and
  sky stages read kept resources by `[material] reads`' rules: `x` for this frame's version of a texture a
  `[[before_world]]` pass writes, `x_previous` for last frame's, and buffers, in their fragments only. The check
  refuses `x` of a texture only `[[post]]` passes write, and `writes` on either stage. An entity or sky stage without
  `reads` composes and draws as before, a test holds. The reference now says that a compute pass may call
  `storageBarrier()`.
- 2026-11-15: API 0.2 (additions, experimental): `[material] reads` (#16011 item 4): the terrain reads kept
  resources, `x` for this frame's version of a texture a `[[before_world]]` pass writes (in the same frame),
  `x_previous` for last frame's, and buffers; their declarations join the material's module at the passes'
  bindings. The check refuses `x` of a texture only `[[post]]` passes write (they run after the terrain draws),
  `writes` on `[material]`, `reads` on the other stages, and `displacement` or a `[vertex]` file reaching a kept
  resource, as both also draw the shadow map. The kept resources' frame (the versions' swap and the clears) now starts
  before the world draws in every pack that keeps them. A material without `reads` composes as before, a test holds.
- 2026-11-15: API 0.2 (addition, experimental): `[frame] fog = "pack"` turns Horizon's fog off on what the material
  and entity stages return, and gives every stage `fn vanilla_fog(position: vec3<f32>) -> f32` and the frame's
  `fog_environment` and `fog_render`, the two distances vanilla's fog takes the larger of, which `fog_start` and
  `fog_end` merge. The enchantment glint Horizon adds after `entity` fades by `vanilla_fog`. A pack without the key
  is drawn and checked as before, and `[material] fog = false` stays as it was; a manifest sets one of the two.
- 2026-11-15: API 0.2 (experimental): `frame.light_direction` is the sun by day and the moon by night also for a pack
  without a `[shadow]` stage, where it now equals the frame's `sun_direction` or `moon_direction`; it read straight up
  (0, 1, 0) at every time of day until now, so a pack that traced its shadows along it got them straight down. Packs
  with `[shadow]` see the same value as before. A fix of a value; no WGSL name changes.
- 2026-11-14: API 0.2 (**breaking**, experimental): the entity stage keeps the engine's coverage unless the pack
  declares it, with `[entity] coverage`, `"engine"` (the default) or `"pack"`. Under `"engine"` the texture's alpha
  times the layer's opacity is cut and blends, not the returned alpha, and `horizon-shader check` refuses a `discard`
  that `entity` can reach. An entity stage that cuts or fades entities itself sets `coverage = "pack"`, which keeps
  the old behaviour.
- 2026-11-14: API 0.2 (experimental): `RAY_ENTITY` also holds the block entities at rest that the block entity cache
  keeps (on in every graphics preset: closed chests, shulker boxes, bells, decorated pots, banners), in view or not,
  so they cast traced shadows and show in reflections as the mobs do. A hit on one is an entity hit like any other.
- **2026-11-13: API 0.1 removed (breaking, #15886).** Use shader API 0.2 instead. The game, `horizon-shader` and
  the registry refuse a 0.1 pack with `needs shader API 0.1; this Horizon serves 0.2`; the game draws without it.
  [Moving a pack to 0.2](#shader-api-02-experimental) is described above; Golden Hour 2.0.0 is the moved example. The
  0.1 reference is gone, and the templates of `horizon-shader new` are 0.2 packs. API 0.2 is unchanged.
- 2026-11-15: API 0.2 (additions, experimental): compute `[[post]]` and `[[before_world]]` files trace under
  `[ray_tracing]`. One that calls a trace function gets `RayHit`, `trace`, `trace_any`, `trace_layers`,
  `trace_any_layers`, `RAY_SOLID`, `RAY_TRANSLUCENT`, `hit_normal_map`, `hit_specular_map` and `block_pbr_maps`, as
  a post pass does; one that calls none, and every pack without the key, composes as before. A `[[before_world]]`
  pass traces this frame's scene. `[ray_tracing]` also takes a `[[before_world]]` pass as what traces. The game runs
  kept resources and compute passes beside `[ray_tracing]`.
- 2026-11-15: API 0.2 (experimental): the game runs kept resources, compute `[[post]]` entries and
  `[[before_world]]` passes (#16011 item 4), which it refused until now. The API is unchanged.
- 2026-11-15: API 0.2 (additions, experimental): fixed-size kept textures (`[[resource]]` with a `size`) may take
  32 MiB together, both versions counted, as much as kept buffers; it was 1 MiB, one 256×256 `rgba16float` texture.
  A camera-centred trail or height map, a light cache or a lookup table can now be a texture rather than a buffer
  indexed by hand: a 1024×1024 `rgba16float` one takes 16 MiB. The device still refuses a side longer than its
  largest texture when the pack loads. Every pack the old budget took passes; no WGSL name changes, so every pack
  builds from the same WGSL as before.
- 2026-11-15: API 0.2 (additions, experimental): post targets are counted by memory. `[[post]] output`s and
  R-relative kept textures together may take 6 render-size `rgba16float` textures, an `output` its `scale`² (1, 0.25
  or 0.0625), an R-relative kept texture 2 as before, with at most 16 `output`s; it was 4 targets, an `output` 1. Two
  kept histories now leave room for a half- and quarter-size bloom pyramid and a full-size target. Every pack the
  count of 4 took passes; no WGSL name changes, so every pack builds from the same WGSL as before.
- 2026-11-14: API 0.2 (additions, experimental): `[motion]`: the post stage of a pack that sets it gets the motion
  guide, `scene_motion` (binding 14), `fn motion_at(texel: vec2<i32>) -> MotionData` and the `MOTION_CAMERA`,
  `MOTION_OBJECT`, `MOTION_NEW` and `MOTION_NO_HISTORY` kinds: how each pixel's surface moved since last frame, for
  temporal passes. `horizon-shader check` takes it; the game refuses a pack with `[motion]` until it draws the guide.
  A pack without the key is built from the same WGSL as before, a test holds; API 0.1 is unchanged.
- 2026-11-14: API 0.2 (experimental): `RAY_ENTITY` also holds the entities within the scene's reach that the camera
  doesn't draw (behind or beside it, or hidden behind terrain), so they cast traced shadows and show in reflections,
  and turning the camera doesn't change the layer. Block entities out of view are not in it yet.
- 2026-11-14: API 0.2 (additions, experimental): under `[ray_tracing]` the scene holds the entities the camera draws in
  a third layer, only for a pack whose files name its constant, `RAY_ENTITY` (outside a comment; dead code counts).
  Such a pack's `trace_layers` and `trace_any_layers` take it, its `RayHit` gains `entity`, and on an entity hit
  `hit_normal_map` and `hit_specular_map` give their defaults; a ray stops where vanilla's entity alpha test keeps the
  pixel. `trace` and `trace_any` keep reading solid terrain alone. The check refuses a pack's own `RAY_ENTITY` or post
  `output` by that name there, and a `displacement` or `vertex` that reaches the layer. Packs that don't name it are
  built from the same WGSL as before, a test holds.
- 2026-11-13: API 0.2 (change, experimental): the game draws entities and block entities through the `[entity]`
  stage (`EntityInput.kind` 0 and 1) instead of refusing a pack that has it, with the stage's shadow maps and coloured
  lights, under either `[frame] blend`. Dropped and framed items, particles and the first-person hand keep Horizon's
  lighting; the reference lists them, and says where `shade` is still 1 with vanilla's shade in `tint`. The stage's
  list of names is held from now on, like the other stages'. No name or field was added. A pack without `[entity]` is
  unchanged.
- 2026-11-13: API 0.2 (additions, experimental, not run yet): `[[before_world]]` (#16011 item 4): compute passes
  once a frame before the world draws, with the keys of a compute `[[post]]` entry, at most 4. Their prelude has the
  frame, the samplers and the shadow maps. Their invocations count toward the 2^26 a frame with the compute
  `[[post]]` entries'. The game refuses a pack with one until a later Horizon runs it.
- 2026-11-13: API 0.2 (additions, experimental, not run yet): compute passes (#16011 item 4). `[[post]]`
  entries with `compute = true` are dispatches in the chain, with `dispatch` (`[x, y, z]` cells or `"pixels"`),
  `workgroup`, `reads` and `writes`, at most 8; an ordinary pass lists the kept resources it `reads`. A compute file
  defines `fn compute(input: ComputeInput)`, and Horizon adds the `@compute` entry with the pass's workgroup size,
  every invocation running and `in_bounds` false past the dispatch. Each pass's prelude declares only the resources it
  lists, at group 3 from binding 64. `horizon-shader check` builds compute files as it builds post passes, refuses a
  file that uses a resource its pass doesn't list, and refuses sampling or gathering an `r32float` or `rgba32float`
  texture (`textureLoad` only). Compute work is capped at 2^25 invocations a dispatch and 2^26 a frame over every
  compute pass, counted as whole workgroups, a `"pixels"` dispatch as 3840×2160. Compute files don't get `trace` yet, and `[material]` can't read kept resources yet:
  both come in later steps. The game refuses a pack with a compute pass until a later Horizon runs it. Packs without
  them are built from the same WGSL as before, a test holds.
- 2026-11-13: API 0.2 (additions, experimental, not run yet): kept resources (#16011 item 4). `[[resource]]`
  declares a buffer (`buffer` bytes of an `element`: `u32`, `atomic<u32>`, `f32`, `vec4<u32>` or `vec4<f32>`) or a
  texture (`format`: `rgba16float`, `r32float`, `rgba32float` or `rgba32uint`, the last read as `texture_2d<u32>` and
  written by compute passes only, never an `output`; a fixed `size` or a `scale` of the render size) a
  pack keeps between frames, `keep = "view"` or `"world"`, the latter fixed in size and with `[kept] reach`. A
  `[[post]] output` may name a kept texture; an option may say what it `resets`. `horizon-shader check` takes them,
  with their names, counts and memory checked (at most 8 resources, 32 MiB of buffers, 1 MiB of fixed textures, and
  R-relative textures two each of the 4 post targets); the game refuses a pack that declares one until a later
  Horizon runs them. Compute passes and the WGSL that reads the resources come next.
- 2026-11-13: API 0.2 (additions, experimental): under `[ray_tracing]` the scene holds translucent terrain (water,
  stained glass, ice) in a second layer. Where `trace` is, the prelude gets
  `fn trace_layers(origin, direction, max_distance, layers) -> RayHit`, `fn trace_any_layers(...) -> bool` and the
  constants `RAY_SOLID` and `RAY_TRANSLUCENT` to name the layers with; `trace` and `trace_any` keep reading solid
  terrain alone, so a picture made with them doesn't change. A hit in the translucent layer has `translucent` true
  and `fluid` 1 on water; a ray stops there at any texel but a clear one (alpha below 0.01), as the translucent pass
  draws. The check refuses a pack's own function, constant or post `output` by the four names there, and a
  `displacement` or `vertex` that reaches the new functions, as `trace`'s. Packs without `[ray_tracing]` are built
  from the same WGSL as before, a test holds.
- 2026-11-13: API 0.2 (**breaking**, experimental): the material stage keeps the engine's coverage unless the pack
  declares it, with `[material] coverage`, `"engine"` (the default) or `"pack"`. Under `"engine"` solid and cutout
  terrain's alpha test reads the texture's alpha, not the returned one, translucent terrain is cut on the texture's
  alpha and blends by the returned alpha, and `horizon-shader check` refuses a `discard` that `material` can reach,
  through its helpers too. A 0.2 material that cuts its own holes sets `coverage = "pack"`, which keeps the old
  behaviour. API 0.1 is unchanged, and refuses the key.
- 2026-11-13: API 0.2 (fix, experimental): `MaterialInput.tangent.xyz` now points where `uv.x` grows, as the
  reference says; it pointed the other way, so a normal map turned into world axes by the reference's rule had both
  in-plane components negated. `w` is unchanged. A 0.2 pack that negated the tangent to make up for it must stop.
  API 0.1 keeps the old direction: its WGSL is frozen.
- 2026-11-13: API 0.2 (additions, experimental): under `[ray_tracing]` `RayHit` gains `emission`, `fluid`,
  `translucent` (false for every hit: the scene holds no translucent terrain yet) and `tangent`, as `MaterialInput` has
  them, and `horizon_sprite`, Horizon's own. Where `trace` is, the prelude gets `hit_normal_map(hit)` and
  `hit_specular_map(hit)`, the resource packs' LabPBR maps at a hit, and a traced post stage gets `block_pbr_maps()`;
  the check refuses a pack's own function or post `output` by those names there, as `trace`'s. Packs without
  `[ray_tracing]` are built from the same WGSL as before, a test holds.
- 2026-11-13: API 0.2 (additions, experimental): under `[ray_tracing]` the material stage gets `RayHit`, `trace` and
  `trace_any` too, the post stage's own, where its files name either function; a material stage that names neither is
  built from the same WGSL as before, a test holds. `[ray_tracing]` needs a `[[post]]` pass or a `[material]` stage.
  `horizon-shader check` builds such a material stage both ways and refuses a pack whose `displacement` or `vertex`
  (both run in vertex shaders) reaches `trace` or `trace_any`.
- 2026-11-12: API 0.2 (additions, experimental): `[ray_tracing]`: the post stage of a pack that sets it gets
  `fn trace(origin, direction, max_distance) -> RayHit` and `fn trace_any(...) -> bool`, which read the terrain around
  the camera where the game traces rays (`HORIZON_RAY_TRACING=1` on a GPU with ray queries) and miss where it doesn't,
  and its `Frame` gets `has_ray_tracing` and `ray_distance`, in what was a gap before `luminance`. `horizon-shader
  check` builds such a post stage both ways and refuses a pack that names a ray query itself. `[[tags]]` and
  `[[materials]]` need no `[material]` stage in such a pack: `RayHit.tags` and `RayHit.material` read them. A pack
  without the key is built from the same WGSL as before, a test holds; API 0.1 is unchanged.
- 2026-11-10: API 0.2 (additions, experimental): the `[entity]` stage, `fn entity(input: EntityInput) -> Surface`,
  for entities, block entities, items and particles, with the material stage's shadow maps and coloured lights. The
  reference gives vanilla's own as the stage's function. `horizon-shader check` takes it; the game refuses a pack
  that has it until it draws the stage. `[shadow]`, `[[lights]]` and `[[particle_lights]]` take `[entity]` as the
  stage that reads them. A pack without `[entity]` is unchanged, and so is API 0.1.
- 2026-11-10: API 0.2 (additions, experimental): `[frame] primaries`, `"bt709"` (the default) or `"bt2020"`, and
  `Frame.primaries` and `luminance` after `black_level`. `horizon-shader check` takes `"bt2020"` with
  `blend = "linear"` only, and the game refuses it for now. The reference's "Room to grow" gains the rule for new
  names: a later release adds a name to a stage's prelude only under a manifest key the pack sets or in a new stage,
  and a test holds each prelude to its list of names. API 0.1 is unchanged: the uploaded `Frame` buffer grows by 16
  bytes after the fields 0.1 declares.
- 2026-11-13: API 0.2 (change, experimental): the game draws 0.2 packs without `HORIZON_HDR_FRAME=float`; the
  switch is gone, and setting it does nothing. HDR is offered only where the device draws the float frame and keeps
  the HUD on a layer of its own. API 0.1 is unchanged: with HDR on, a 0.1 pack keeps the frame it had.
- 2026-11-09: API 0.2 (change, experimental): the game draws `[frame] blend = "linear"` packs instead of refusing
  them: the frame holds linear light, so see-through draws blend as light does, and a stage sees the same values
  as under `"gamma"`. API 0.1 is unchanged.
- 2026-11-08: API 0.2 (change, experimental): with `HORIZON_HDR_FRAME=float` and HDR on, where the device keeps the
  HUD on a layer of its own, a 0.2 pack is shown in HDR: `Frame`'s display fields are the display's, its `[tonemap]`
  or else Horizon's no-pack curve maps the light once, the HUD is laid over it as without a pack, and F2 writes the
  same SDR screenshot and `-hdr.png` as without one. With HDR off or no HUD layer the fields stay SDR's and the
  picture ends on an 8-bit frame before the camera overlays and the HUD. The reference says Horizon lifts the shadows
  onto `black_level` after `[tonemap]`. API 0.1 is unchanged.
- 2026-11-08: API 0.2 (change, experimental): `horizon-shader check` refuses a 0.2 pack that constructs `Frame`, a
  stage input (`PostInput(...)`) or `SurfaceData`, which may gain fields; reading and passing them along are
  unchanged, and `Surface` is still constructed. The reference says `[tonemap]` runs once per pixel per output. API
  0.1 is unchanged.
- 2026-11-08: API 0.2 (fix, experimental): the world a post pass reads holds coverage in its alpha, as the
  reference says, instead of Horizon's light-source marks. API 0.1 is unchanged.
- 2026-11-07: API 0.2 (change, experimental): with `HORIZON_HDR_FRAME=float`, a 0.2 pack picked or dropped gets its
  frame at once instead of at the next start. API 0.1 is unchanged.
- 2026-11-05: API 0.2 (change, experimental): the game draws 0.2 packs with `HORIZON_HDR_FRAME=float` set, on a
  float frame where the device can; `[frame] blend = "linear"` is still refused. API 0.1 is unchanged.
- 2026-11-05: API 0.2 (new line, experimental): linear light, `Frame`'s display fields, `[tonemap]` and
  `[frame] blend`, with its own reference. `horizon-shader` checks and packs it; the game and the registry refuse it
  for now. API 0.1 is unchanged: the uploaded `Frame` buffer grows by 32 bytes after the fields 0.1 declares.
- 2026-11-04: API 0.1 (change, experimental): the post stage's `scene_surface` is `texture_2d<u32>`, the bits of
  the stored floats (RGBA32F is not renderable on OpenGL). Read it through `surface_at`, or
  `bitcast<vec4<f32>>(textureLoad(scene_surface, p, 0))`; a `textureLoad` as `vec4<f32>` no longer validates.
- 2026-11-03: API 0.1 (no change to packs): float and int `[[options]]` are sliders on the options page, on their
  `step`; the value is saved, and the pack rebuilt, when the slider is let go.
- 2026-11-03: API 0.1 (additions): `[shadow] option`, `resolution_option` and `distance_option`: a pack's
  options can turn its shadow map off (no map, no shadow pass, `has_shadow` 0) and pick its size, so
  one pack offers its own quality levels. The options sizing the map need no `override`.
- 2026-11-03: API 0.1 (additions): with `[shadow]`, `[displacement]`'s grid casts into the shadow map in
  the flat faces' place, so the relief shadows itself.
- 2026-11-03: API 0.1 (additions): `[displacement]` draws: near the camera the faces with LabPBR relief are a
  grid moved by `displacement`, and `MaterialInput.displaced` says how much of it a point has.
- 2026-11-03: API 0.1 (additions): the manifest's `[displacement]`, the material stage's
  `fn displacement(input: DisplacementInput) -> f32` and `MaterialInput.displaced`.
- 2026-11-02: API 0.1 (additions): with dynamic lights on, held and dropped items light
  `MaterialInput.colored_light` in their block's `[[lights]]` colour.
- 2026-11-02: API 0.1 (additions): `[[particle_lights]]` and `MaterialInput.colored_light`.
- 2026-11-01: API 0.1 (additions): `[[materials]]` and `MaterialInput.material`, an id by sprite,
  and `[shadow] solid_materials`.
- 2026-11-01: API 0.1 (additions): entities cast into `shadow_map`, the player's own model in first person too, and the round
  blob under entities is left out in the Overworld while a pack draws a shadow map, so under a
  roof they have no shadow but what the pack gives them. Only entities the camera's culling keeps
  cast: one off screen or behind a wall leaves the map, and its shadow with it.
- 2026-11-01: API 0.1 (additions): `[sky] clouds`, `sun` and `stars`: a sky can replace Minecraft's.
- 2026-11-01: API 0.1 (additions): `[[pages]]`, the layout of a pack's options page.
- 2026-11-01: API 0.1 (additions): the `[weather]` stage
  (`fn weather(input: WeatherInput) -> vec4<f32>`) colours rain and snow streaks.
- 2026-11-01: API 0.1 (additions): `[[textures]]`, PNGs bound for every stage.
- 2026-11-01: API 0.1 (additions): `[[lights]]` and the material stage's `block_light_color`.
- 2026-10-31: API 0.1 (additions): translucent terrain in the shadow pass: `shadow_map_translucent`
  and `shadow_color`, so a pack can tint the light that passes through water and stained glass.
- 2026-10-31: API 0.1 (additions): `Frame.mods`, eight `vec4<f32>` a native mod hands the pack it bundles each
  frame (`native_shader_params` in docs/modding/src/how-to/native-mods.md), all 0 when no mod sets them.
- 2026-10-31: API 0.1 (additions): `[shadow]`: the engine draws the solid terrain the light sees
  into a distorted depth map each frame, which the material and post stages read as `shadow_map`
  with `shadow_sampler` and the prelude's `shadow_position`; the frame's `light_direction`,
  `has_shadow`, `shadow_view_proj`, `shadow_distance`, `shadow_distortion`, `shadow_resolution`
  and `shadow_center_texel`.
- 2026-10-30: API 0.1 (additions): the manifest's `[reflection] option`, which draws the water
  mirror only while that option is on, and the mirror's own eye in facing culling.
- 2026-10-30: API 0.1 (additions): the material stage's `reflection` and
  `reflection_sampler`, the world mirrored about the water plane around the camera, drawn
  only for a pack whose material reads it; `Frame.water_plane`, `has_reflection` and
  `reflection_resolution`.
- 2026-10-30: API 0.1 (additions): `MaterialInput.face`, `fluid`, `emission`, `normal` and
  `tangent`: which fluid and face a point is on, how strongly its block shines, and the
  surface's frame for normal maps and lighting.
- 2026-10-30: API 0.1 (additions): the post stage's `history`, last frame's input to the last
  pass, so a temporal pass second to last reads its own previous output (TAA, denoising).
- 2026-10-30: API 0.1 (additions): `Frame.previous_view_projection`, `camera_motion` and
  `delta_time` for temporal passes (reprojection, motion blur); `eye_light_smooth`, `in_water`
  and `in_lava`, eased so exposure and underwater fog fade instead of stepping.
- 2026-10-30: sprite tags (`[[tags]]`, `MaterialInput.tags`) and the frame's `precipitation`, `temperature`,
  `downfall` and `wetness`.
- 2026-10-30: API 0.1 (additions): the material stage's `MaterialInput.screen` and its
  `opaque_depth` and `opaque_color`, the world as it was before translucent terrain drew, so
  water and glass can see what lies behind them (depth under water, refraction).
- 2026-10-26: the material stage in the game: terrain's surfaces through the pack's `material`.
- 2026-10-26: the sky stage in the game: the pack's `sky` gets vanilla's colour along each ray.
- 2026-10-26: the post stage in the game (`HORIZON_SHADER_PACK`, hot reload with the last good pack kept).
- 2026-10-25: `horizon-shader` (`new`, `check`, `pack`), three templates, the example pack and the guide.
- 2026-10-25: the check (naga validation, signatures, options, budgets, SPIR-V/Metal/HLSL translation).
- 2026-10-25: the stages of API 0.1 (sky, material, post), their inputs and the generated reference.
- 2026-10-25: the pack format, schema 1: `horizon-shaders.toml` and the `.hshader` package.
- 2026-10-24: the earlier shader pack code was removed; this API is designed from scratch.
- 2026-11-01: API 0.1 (additions): a stage's `files`.
- 2026-11-01: API 0.1 (additions): `[material] fog`.
- 2026-11-01: API 0.1 (additions): the material stage reads the resource packs' LabPBR maps (`_n`, `_s`): `block_pbr_maps`, `block_normal_map`, `block_specular_map`, their `_level` forms and `block_pbr_size`. The maps load only while a native pack is picked: without one they cost nothing, and the first pick reloads the resources once.
- 2026-11-01: API 0.1 (additions): `[[post]] output` and `scale`, targets of a pack's own at the world's size, half or a quarter of it.
- 2026-11-01: API 0.1 (additions): the material stage's `block_texture`, `block_texture_level`, `block_texture_size` and `block_texture_lod`, the surface's own sprite anywhere on it. The LabPBR `_level` functions now count the block texture's mip levels, the same footprint where a map is smaller.
- 2026-11-01: API 0.1 (additions): the material stage's `set_surface` and the post stage's `surface_at` and `scene_surface`: a normal, a smoothness and a value of the pack's own per pixel of solid terrain. Only a material stage that calls `set_surface` pays for the target.
- 2026-11-01: API 0.1 (additions): `[[materials]] blocks`, ids by block, and `translucent`, moving an id's sprites into the translucent pass. `sprites` may be left out when `blocks` is there.
- 2026-11-01: API 0.1 (additions): the `[vertex]` stage, `fn vertex(input: VertexInput) -> vec3<f32>`: waving plants and rolling water, in the world and the shadow map.
