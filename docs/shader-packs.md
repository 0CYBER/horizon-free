# Shader packs

Horizon will run two kinds of shader packs:

- **Horizon shader packs**: WGSL, a `pack.toml` manifest and documented stages. Horizon's own format, made for wgpu,
  with hot reload while you edit.
- **Java shader packs** (OptiFine/Iris): the `shaders/` packs players already have, translated from GLSL to WGSL
  when they load.

Both are off by default. With no pack selected the renderer draws exactly what it draws today, which is vanilla
26.3's picture: no extra pass, no extra target, the same pixels.

| Part | State |
| --- | --- |
| Renderer layout (below) | today's code |
| Horizon shader packs: `composite` stage, uniforms, hot reload, Shader Packs page | done (the page: list, pick, open the folder, drop a pack on it) |
| Horizon shader packs, api 2: stage `format`s, `[profiles]`, option descriptions, a `noise` texture, the opaque depth, a G-buffer reconstructed from depth, a sun/moon shadow map (Stages, below) | first slice: `examples/shaderpacks/horizon-ultra` |
| Horizon shader packs: `deferred`, `gbuffer`, `shadow` stages (the pack's own code in the world pass) | planned |
| Shader Pack Settings page: a pack's options, sub-screens, sliders, profiles and Reset, Horizon and Java packs alike; applied once on Done, as Iris does | done |
| Java shader packs: preprocessing + GLSL parse coverage | spike done: 450 of 470 programs reach valid WGSL (below) |
| Java shader packs: preprocessor (`crates/horizon-shaderpack`: includes, options, Iris's macros, JCPP's `#if` and `#define` rules, `.lang` files) | done: every program of the three test packs preprocesses; ten packs tested (Compatibility, below) |
| Java shader packs: GLSL → WGSL (`horizon_shaderpack::translate`: syntax-tree legaliser, naga; struct and nested-array varyings, vertex `texture`, `mat2(vec4)`, `1 << 31`) | done: the 464 programs the three test packs enable by default translate (below) |
| Java shader packs: `shaders.properties` (`horizon_shaderpack::properties`: screens, sliders, profiles, `program.*.enabled`, image formats, blend modes, `texture.*`) | done: read like Iris for the three test packs |
| Java shader packs: full-screen pass plan (`horizon_shaderpack::pipeline`: `DRAWBUFFERS`/`RENDERTARGETS`, `colortexN` formats and clearing, buffer flips, `blend.*`) | done: planned for the three test packs |
| Java shader packs: full-screen passes drawn (`java_pack.rs`, `gfx/java_stages.rs`: `begin`, `prepare`, `deferred*`, `composite*`, `final` over Horizon's world) | first slice: BSL and Complementary run (below); `depthtex1`/`depthtex2` before translucent geometry, `blend.*`, PNG `texture.<stage>.*` and `texture.noise` |
| Java shader packs: custom uniforms (`horizon_shaderpack::uniforms`), `sunPathRotation`, `clouds`/`sun`/`moon`, eye light, `wetness` | done for the full-screen passes (below) |
| Java shader packs: `gbuffers_terrain`, `gbuffers_water` (`gfx/java_terrain.rs`: Horizon's terrain fed to the pack's programs) | first slice: MakeUp Ultra Fast, Chocapic13' and BSL draw their own terrain and water (below) |
| Java shader packs: `shadow` programs (`gfx/java_stages.rs`: the shadow map from Horizon's terrain) | first slice: terrain shadows for MakeUp Ultra Fast, Chocapic13' and BSL (below) |
| Java shader packs: other `gbuffers_*`, entity shadows, `shadowcomp`, `.dat` and resource-location textures, images, compute | planned |

## How the renderer draws a frame today

Everything is in `crates/horizon-client/src/gfx.rs`, `Renderer::render`, one command encoder per frame:

1. **World pass** (`"Horizon world"`): clears colour to the fog colour (black on the title screen and in the End)
   and depth to 0 (reversed depth, `Depth32Float`, `GreaterEqual`; `camera::perspective`). In order: the sky
   (`sky` pipeline, `terrain.wgsl`), the title panorama (`gfx/panorama.rs`) or the sun, moon and stars
   (`celestial_pipeline`), the dark disc; solid and cutout terrain (`solid`, one indirect multi-draw over the
   section arena, `gfx/arena.rs`); entities and block entities (`entity_pipeline`, `entity.wgsl`, batched by
   `gfx/entity_batch.rs`); block breaking (`crumbling_pipeline`); additive geometry (lightning, dragon beams);
   the block outline and debug lines; translucent terrain (`translucent`); the world border (`border.wgsl`);
   translucent crumbling; clouds (`cloud_pipeline`). Weather and particles are entity quads. Depth is
   discarded at the end of the pass.
2. **World resolve** (`gfx/world_post.rs`, `world_post.wgsl`): only when resolution scale isn't 100% or FXAA is
   on, the world pass draws into its own colour and depth targets, and one full-screen pass copies or
   anti-aliases them onto the screen.
3. **Hand** (`"First-person hand"`): the first-person hand and held item, with depth cleared, at the hand FOV.
4. **Screen effects** (`screen_effects.rs`): pumpkin blur, portal, powder snow, vignette, nausea, as
   full-screen quads.
5. **Menu blur** (`menu_blur.rs`, `menu_blur.wgsl`): vanilla's `menuBackgroundBlurriness` behind screens.
6. **HUD** (`hud.rs`, `hud.wgsl`): GUI, text, items in slots.

All world shaders share one uniform block, `Globals` in `gfx.rs` (`struct Globals` in `terrain.wgsl`):
view-projection, inverse view, fog colour and distances, sky colour, sun direction, sunrise colour, game time.
The lightmap, fog and sky colours are computed on the CPU (`environment.rs`, `fog.rs`) as vanilla does.

## Horizon shader packs

### Where packs live and how one is picked

- Packs go in `shaderpacks/` beside `options.txt` in the game folder (the same folder Iris uses, so one folder
  holds both kinds). That is the game home the launcher starts the game in (`%APPDATA%\horizon` on Windows,
  `~/Library/Application Support/horizon` on macOS, `~/.local/share/horizon` on Linux), not the launcher's version
  folder. A pack with a `pack.toml` at its root is a Horizon pack; a folder with `shaders/`, or a `.zip`, is a Java
  pack.
- The **Shader Packs** page lists them: **Video Settings → Shader Packs...** (where Iris puts its button), **Options →
  Horizon Settings... → Shader Packs...**, and the Shader Pack row of the graphics settings lead to it. It shows the folder's full path
  and an *Open Folder* button (the folder is made if it is missing), then one switch per pack: turning one on picks
  it, turning it off is the vanilla picture. With no pack, it says so under the path. The list is read again every
  second while the page is open, so a pack dropped into the folder shows up. A `.zip` or pack folder dropped on the
  window while the page shows is copied into the folder and picked (Iris's `ShaderPackScreen.onFilesDrop`); one of
  that name already there is left alone.
- With a pack on that has options, **Shader Pack Settings...** opens them: Horizon packs' `[options]` and
  `[profiles]`, Java packs' `#define`/`const` options laid out by `shaders.properties` (`screen`, `screen.<name>`,
  `sliders`, `profile.*`) and named by `lang/en_us.lang` (`option.*`, `value.*`, `screen.*`, `profile.*`, as Iris;
  without one, the option's own name). The values are saved and the pack built again once, when the page closes
  (Iris applies on Done), not on every click or slider step. A saved value an option doesn't list is ignored.
- The choice is kept in `horizon-graphics.txt` (`shaderPack:<folder or zip name>`, empty = off). A pack's option
  values are kept in `shaderpacks/<pack>.txt`, as Iris does.
- A pack that fails to load or compile leaves the vanilla picture (or the last pack that worked) on and says why on
  its row ("Failed", with the error), in chat and in the log. Nothing a pack does can stop the game from starting.
- Benchmarks (`--benchmark`) and the parity harnesses never load a pack.

### Layout

```
my-pack/
  pack.toml
  shaders/
    composite.wgsl
    bloom.wgsl
  textures/
    noise.png          # api 2: [textures] noise
  LICENSE
```

```toml
[pack]
name = "Warm evenings"
description = "A gentle grade and a soft vignette."
authors = ["you"]
version = "1.0.0"
license = "MIT"
api = 1                        # the stage and uniform contract below

[[stage]]
kind = "composite"             # see Stages
shader = "shaders/composite.wgsl"
entry = "fs_main"              # optional, default fs_main

[[stage]]
kind = "composite"             # stages of one kind run in file order
shader = "shaders/bloom.wgsl"
scale = 0.5                    # optional: this stage's target size relative to the screen

[options.saturation]
label = "Saturation"
default = 1.15
min = 0.0
max = 2.0
step = 0.05

[options.vignette]
label = "Vignette"
default = true
description = "Darkens the corners."   # optional: the button's tooltip
```

Api 2 (`api = 2`) adds:

```toml
[[stage]]
kind = "composite"
shader = "shaders/lighting.wgsl"
format = "rgba16float"         # this stage's target: rgba8unorm, rgba16float or rgb10a2unorm; the last stage
                               # always writes the screen's format, so earlier ones can keep HDR colour

[profiles.low]                 # the Profile button cycles these; "Custom" when the values match none
vignette = false
saturation = 1.0

[textures]
noise = "textures/noise.png"   # replaces the prelude's noise_texture (256x256 random RGB without it)
```

Options become WGSL [pipeline-overridable constants](https://www.w3.org/TR/WGSL/#override-decls): the shader
declares `override saturation: f32 = 1.15;` and `override vignette: bool = true;`, and the settings screen sets
them. Numbers are `f32` (with `min`/`max`/`step`, a slider) or `i32` (`values = [...]`, a cycle button); `true`/`false`
are `bool` (an on/off button). Changing an option rebuilds that stage's pipeline (milliseconds), which is the job
OptiFine's `#define` options do, without text substitution.

### Stages

A stage is one full-screen fragment shader. Horizon supplies the vertex shader (one triangle) and, before the
pack's source, a prelude with the bindings, the `Frame` uniform and helper functions, so a pack's file holds only
its own code.

| Stage | Runs | Reads | Writes | State |
| --- | --- | --- | --- | --- |
| `composite` | after the world pass, before the hand | scene colour and depth, previous stage | the next scene colour | first slice |
| `final` | after the hand and screen effects, before menus and HUD | colour with hand, depth | the screen | planned |
| `deferred` | after opaque terrain and entities, before translucent ones | opaque colour, depth, G-buffers | opaque colour | planned |
| `gbuffer` | terrain, entities, water, sky: a WGSL `material` function called by Horizon's shaders | vertex data, atlas | colour + G-buffers (normal, material) | planned |
| `shadow` | the world from the sun, into a shadow map | vertex data | shadow depth | planned |

The first slice has `composite` only. A `composite` stage reads the scene, not the HUD, the hand or the menus,
so a pack can't make text unreadable. Every stage after the first also reads the previous stage's output.

Bindings in group 0 (the prelude declares them):

| Binding | WGSL | What |
| --- | --- | --- |
| 0 | `scene_color: texture_2d<f32>` | the world's colour (sRGB-encoded, as on screen) |
| 1 | `scene_depth: texture_depth_2d` | the world's depth (reversed: 1 near, 0 sky) |
| 2 | `previous: texture_2d<f32>` | the previous `composite` stage's output; `scene_color` for the first |
| 3 | `linear_sampler: sampler` | linear, clamp to edge |
| 4 | `nearest_sampler: sampler` | nearest, clamp to edge |
| 5 | `frame: Frame` (uniform) | below |

```wgsl
struct Frame {
    view: mat4x4<f32>,              // world (camera-relative) to view
    projection: mat4x4<f32>,        // view to clip, reversed depth
    inverse_view: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    camera_position: vec4<f32>,     // xyz: world position of the eye (f32, so ~1 mm precision at 30 000 blocks)
    resolution: vec4<f32>,          // width, height, 1/width, 1/height of this stage's target
    time: vec4<f32>,                // seconds since the pack loaded, frame counter, day fraction (0 = sunrise), partial tick
    sun: vec4<f32>,                 // xyz: direction to the sun (world space), w: sun angle in radians (0 at noon)
    moon: vec4<f32>,                // xyz: direction to the moon, w: moon phase (0..7)
    sky_color: vec4<f32>,           // vanilla's sky colour this frame
    fog_color: vec4<f32>,           // vanilla's fog colour this frame (what the world pass clears to)
    fog: vec4<f32>,                 // environmental fog start, end; sky fog end; cloud fog end (blocks)
    weather: vec4<f32>,             // rain level, thunder level, 0, 0 (smoothed as vanilla does)
    camera_state: vec4<f32>,        // submerged in: 0 air, 1 water, 2 lava, 3 powder snow; night vision; darkness; blindness (0 for now)
    view_params: vec4<f32>,         // near, far (Camera.depthFar), render distance (blocks), vertical FOV (radians)
    dimension: vec4<f32>,           // 0 overworld, -1 nether, 1 end; has sky light; 0, 0
}
```

Prelude helpers: `linear_depth(depth) -> f32` (blocks from the eye), `view_position(uv, depth) -> vec3`,
`world_offset(uv, depth) -> vec3` (camera-relative world position), `is_sky(depth) -> bool`, `luminance(rgb)`,
`srgb_to_linear` / `linear_to_srgb`. They hide reversed depth, so a pack doesn't depend on it.

The uniform layout only grows: a new field goes at the end and the `api` number goes up; a pack that declares an
older `api` still loads.

An `api = 2` pack's stages get a second prelude after the first (`shader_pack_prelude_api2.wgsl`; api 1 packs don't,
so names they declare themselves can't clash):

| Binding | WGSL | What |
| --- | --- | --- |
| 6 | `frame_ext: FrameExt` (uniform) | last frame's view and projection and camera, the shadow map's matrices, resolution and distance, eye light, smoothed rain |
| 7, 8 | `scene_normal`, `scene_material: texture_2d<f32>` | the G-buffer (below) |
| 9 | `opaque_depth: texture_depth_2d` | the world's depth before translucent geometry (water, glass, ice) |
| 10, 11 | `shadow_depth: texture_depth_2d`, `shadow_sampler: sampler_comparison` | the sun's (the moon's at night) shadow map |
| 12, 13 | `noise_texture: texture_2d<f32>`, `repeat_sampler: sampler` | `[textures] noise`, linear, repeat |

Helpers: `opaque_depth_at(uv)`, `world_normal_at(uv)`, `material_at(uv)` (block light, sky light, translucent mask,
roughness), `reproject_uv(world_offset)` (last frame's uv and depth), `sample_shadow(world_offset, normal)` (5-tap
PCF, 1 lit .. 0 shadowed), `aces_tonemap(rgb)`.

What each costs, only while a stage reads it (naga reports the bindings each entry point uses): the opaque depth, a
depth copy between the opaque and translucent halves of the world pass; the G-buffer, one full-screen pass after
the world; the shadow map, a depth pass of the terrain and entities from the light. Limits of this first slice: the
G-buffer is reconstructed from depth and colour (normals from neighbouring depths, block light estimated from
brightness, sky light the camera's), not written by the world's shaders; the shadow map holds the sections the
camera draws, so casters outside the view throw no shadow, and none of the terrain where Metal draws sections as one
list (`gfx/flat.rs`, more than 4096 of them). The `deferred`, `gbuffer` and `shadow` stages above replace these.

### How it renders

With a pack on, the world pass draws into the world target of `gfx/world_post.rs` (it exists already for resolution
scale and FXAA), with its depth stored and bindable. Then each `composite` stage draws into a ping-pong colour
target, and the last output goes through the usual copy or FXAA onto the screen, under the hand, screen effects,
menus and HUD. Without a pack none of this exists, and the frame is today's frame.

### Hot reload

While a pack is selected Horizon checks its files' modification times twice a second (a folder pack; a zip reloads
when the zip changes). A change reparses `pack.toml`, compiles every stage with naga first (parse and validate, with
the prelude), and swaps the pipelines only if all of them compiled; otherwise the last good pipelines stay on and the
error (file, line, column, message) goes to chat and the log. F3 + T (vanilla's resource reload) reloads the pack
too.

### Safety

A Horizon shader pack is WGSL only: it runs on the GPU, can't read files, the network or game memory, and naga
validates it before wgpu sees it. A shader can still be slow or loop for long enough to make the GPU driver reset;
that costs a frame or a device loss, which the renderer already handles. Limits: 16 stages, 64 options, stage
targets at most the screen size times 2.

## Java shader packs (OptiFine/Iris)

### What a Java pack is

A `shaders/` folder of GLSL programs, named by where Iris (and OptiFine before it) runs them, plus
`shaders.properties`:

- `shadow*`: the world from the sun into `shadowtex0/1` and `shadowcolor0/1`.
- `gbuffers_*`: every kind of geometry (`basic`, `textured`, `textured_lit`, `skybasic`, `skytextured`, `clouds`,
  `terrain` (+ `_solid`, `_cutout`), `damagedblock`, `block`, `beaconbeam`, `item`, `entities`,
  `entities_glowing`, `armor_glint`, `spidereyes`, `hand`, `weather`, `water`, `hand_water`, `line`), with a
  fallback chain (a missing `gbuffers_terrain` uses `gbuffers_textured_lit`, then `gbuffers_textured`, then
  `gbuffers_basic`). They write up to 16 colour targets (`colortex0..15`, older names `gcolor`, `gdepth`,
  `gnormal`, `composite`, `gaux1..4`).
- `deferred*` (after opaque geometry), `composite*` (after translucent), `final` (to the screen), and the
  compute programs `prepare`, `begin`, `setup` (`.csh`), all full-screen.
- In the source: `/* DRAWBUFFERS:024 */` or `/* RENDERTARGETS: 0,2,4 */`, `const int colortex0Format = RGBA16F;`,
  `const int shadowMapResolution = 2048;`, `const float sunPathRotation = -40.0;` and many more constants Iris reads.
- `shaders.properties`: the option screens and profiles, `program.<name>.enabled`, blend modes, custom textures
  (`texture.composite.colortex4 = ...`), custom uniforms (`uniform.float.x = <expression>`), per-dimension folders
  (`world0`, `world-1`, `world1`); `block.properties`, `item.properties` and `entity.properties` map block, item
  and entity ids to the numbers the shaders see in `mc_Entity`.
- Options are `#define`s in the source (`#define SHADOW_QUALITY 2 // [1 2 3]`, `//#define BLOOM`), found by
  scanning the files.

### Translation

1. **Preprocess like Iris.** Resolve `#include` (absolute from `shaders/`, or relative), apply the user's option
   values to the `#define` lines, and define what Iris defines (`MC_VERSION`, `IS_IRIS`, `MC_GL_VERSION`,
   `MC_GLSL_VERSION`, `MC_OS_*`, `MC_GL_VENDOR_*`, `MC_GL_RENDERER_*`, `MC_RENDER_QUALITY`,
   `MC_SHADOW_QUALITY`, `MC_HAND_DEPTH`, the `DH_*`/feature flags as unsupported).
2. **Legalise to GLSL 450 core.** Packs are written for `#version 120`..`460 compatibility` and use the
   compatibility profile: `gl_FragData`/`gl_FragColor`, `varying`/`attribute`, `texture2D`/`shadow2D`/
   `texture2DLod`, `gl_Vertex`, `gl_MultiTexCoord*`, `gl_Color`, `gl_Normal`, `gl_ModelViewMatrix`, `ftransform()`,
   loose `uniform float x;`. Iris rewrites the same things for the core profile (glsl-transformer). The legaliser
   turns them into explicit `layout(location)` inputs and outputs, one uniform block per stage, and
   separate texture + sampler bindings, which naga's GLSL front end (`naga::front::glsl`, Vulkan-style GLSL 440+)
   accepts.
3. **naga GLSL → IR → WGSL**, validated, cached per pack, options and Horizon version under the game folder (only
   the translation, never the pack's files).
4. **Emulate the pipeline.** Allocate the colour targets with the formats the pack asks for, run the programs in
   Iris's order with its blend and clear rules, feed the uniforms Iris provides (about a hundred:
   `gbufferModelView`, `gbufferProjection`, their inverses and previous-frame versions, `shadowModelView`,
   `shadowProjection`, `cameraPosition`, `previousCameraPosition`, `sunPosition`, `moonPosition`,
   `shadowLightPosition`, `sunAngle`, `worldTime`, `worldDay`, `moonPhase`, `frameTimeCounter`, `frameCounter`,
   `viewWidth`, `viewHeight`, `aspectRatio`, `near`, `far`, `rainStrength`, `wetness`, `eyeBrightness(Smooth)`,
   `isEyeInWater`, `nightVision`, `blindness`, `darknessFactor`, `fogColor`, `skyColor`, `heldItemId`,
   `heldBlockLightValue`, `biome`, ...), and give `gbuffers_*` the vertex attributes they read (`mc_Entity`,
   `mc_midTexCoord`, `at_tangent`, `at_midBlock`, lightmap coordinates, normals).

### What comes first, what is hard

| Piece | Difficulty | Why |
| --- | --- | --- |
| Preprocessor, options, `shaders.properties` screens | easy | text; no GPU |
| Legaliser + naga parse | medium | many legacy built-ins and implicit conversions; the spike measures it |
| `composite*` / `final` / `deferred*` | medium | full-screen passes, like Horizon's `composite`; need the colour targets and uniforms |
| Uniforms | medium | most exist on the CPU already; previous-frame matrices, `eyeBrightnessSmooth`, `wetness` need tracking |
| `gbuffers_*` for terrain | hard | Horizon's packed terrain vertices and arena multi-draw must also feed Iris's attributes and up to 8 render targets; `block.properties` ids per block state |
| `gbuffers_*` for entities, hand, weather, sky, clouds | hard | each is its own Horizon pipeline today; each needs an Iris-shaped twin |
| `shadow` | hard | a second world pass from the sun with its own culling (Horizon culls from the camera) and the pack's distortion in its vertex shader |
| Custom textures, `noisetex` | easy | image loads |
| Custom uniforms (`uniform.*` expressions) | medium | a small expression evaluator per frame |
| Compute (`.csh`), `image2D` load/store | medium | wgpu has compute; storage formats are more limited than OpenGL's |
| Distant Horizons, Voxy hooks, `DH_*` | not planned | Horizon's native LOD terrain shares world colour/depth; it does not expose these mods' APIs |

Order: preprocessor and legaliser until the three test packs parse; then `composite`/`final`/`deferred` with the
uniforms; then `gbuffers_terrain`, `gbuffers_water` and `gbuffers_skybasic`/`skytextured` (the packs' look comes
from these); then entities, hand, weather, clouds; then `shadow`; then custom textures, compute. Custom uniforms
are done (below).

The test set is Complementary Reimagined, BSL and Sildur's Vibrant. Their licences don't allow redistribution, so
they are downloaded for development only, outside the repository (never committed, never shipped, never uploaded);
players bring their own, as with Iris.

## Spike results

Run on 2026-10-15 with naga 30.0.1 over Complementary Reimagined r5.9.3, BSL v10.1.8 and Sildur's Vibrant v2.02
Extreme (the 26.3 builds from Modrinth, kept outside the repository). Every program of the overworld, Nether and End
folders and the shared root was counted; the `dh_*`, `clrwl_*` and `voxy*` programs, which Iris only loads with
Distant Horizons, Colorwheel or Voxy installed, were left out.

| Pack | Programs | naga as is | Preprocessed + legalised: parse | validate | WGSL written |
| --- | ---: | ---: | ---: | ---: | ---: |
| Complementary Reimagined | 183 | 0 | 180 | 180 | 180 |
| BSL | 173 | 0 | 170 | 165 | 165 |
| Sildur's Vibrant | 114 | 0 | 114 | 105 | 105 |
| **Total** | **470** | **0** | **464** | **450** | **450 (96%)** |

"As is" is naga's GLSL front end on the files with `#include` inlined and Iris's macros defined: it accepts only
`#version 440`–`460` with Vulkan-style bindings, and every pack is written for `#version 120`–`330` compatibility
profiles. "Preprocessed + legalised" is the translation of steps 1–3 above, done by a throwaway token-level tool:
Iris's `#include`, its macros (`MC_VERSION`, `MC_RENDER_STAGE_*`, ...) and the packs' `#if`s expanded with naga's own
preprocessor (`pp-rs`), then:

- `#version 450 core`; `attribute`/`varying` to `in`/`out` with locations, one per declarator;
- loose uniforms into one `std140` block (`bool`s stored as `int`);
- `sampler2D` & co. split into texture + sampler bindings, uses rebuilt as `sampler2D(t, s)`, also through function
  parameters, and not where a local of the same name hides the uniform;
- compatibility built-ins (`gl_Vertex`, `gl_MultiTexCoord*`, `gl_Color`, `gl_ModelViewMatrix`, `gl_Fog`, `ftransform()`,
  `gl_FragColor`, `gl_FragData[n]`) and Iris's core-profile names (`vaPosition`, `modelViewMatrix`, ...) declared;
  `texture2D`, `shadow2D`, `texture2DLod`, ... renamed;
- global variables with non-constant initialisers (accepted by NVIDIA and AMD drivers, used throughout
  Complementary) declared at global scope and assigned at the start of `main`; unsized global arrays sized from
  their initialiser;
- `texture(shadowSampler, ..).x`: the float widened to `vec4` first (naga has no scalar swizzle);
- `isnan`, `isinf` and `modf` replaced by helper functions (WGSL has no `isnan`/`isinf`).

What is left, by cause:

| Programs | Cause | Fix |
| ---: | --- | --- |
| 9 | Sildur's passes a `mat3` between stages (`tbnMatrix`); WGSL can't | split matrix varyings into columns |
| 6 | `writeonly image2D` without a format (`shadowcomp.csh` in Complementary and BSL) | take the format from the pack's image declaration (`shaders.properties` / `const int` format) |
| 5 | naga GLSL front-end bug: a call result inside a constructor (`vec3(TAAJitter(..), z)`) leaves an unvisited expression (BSL) | spill call results to temporaries; report to naga |

The translator in `crates/horizon-shaderpack` now passes all of them that the packs enable by default (464 of 464,
the WGSL read back and validated): matrix varyings are split into columns; the five BSL failures were not a naga bug
but dead code after a `discard;` (how BSL turns its clouds and weather off), which the legaliser drops; the six
`shadowcomp.csh` are off by default (`program.*/shadowcomp.enabled` in `shaders.properties`) and use images and arrays
that only exist with their options on, so they wait for `shaders.properties`.

With `shaders.properties` read (`program.*.enabled` evaluated like Iris, image formats taken from `image.*`), the
programs the packs enable translate: 180 of 180 for Complementary, 114 of 114 for Sildur's and 154 of 154 for BSL with
its profile (it turns `composite2`/`composite3` off). With their colored lighting on, BSL's `shadowcomp.csh` translates
too, and so do all 183 of Complementary's on Linux once `IRIS_FEATURE_CUSTOM_IMAGES` is defined: without it, the pack
declares no voxel images but its `shaders.properties` still turns `shadowcomp` on. Which `IRIS_FEATURE_*` Horizon
defines is decided when the stage that backs them exists.

So the GLSL side is feasible: naga's front end plus a legaliser reaches 96% on the three packs, and every remaining
failure has a mechanical fix. Valid WGSL isn't a working pack yet: the output still has to be bound to Horizon's
pipeline (render targets, the uniforms' values, vertex attributes for `gbuffers_*`), which is the emulation work.
The spike also showed what the real translator needs that a token rewriter does badly: scopes (a local named like a
uniform), types (which `texture` call returns a float) and declarations split across `#if`s, so it should rewrite a
GLSL syntax tree after preprocessing, not tokens.

### Running a Java pack's full-screen passes

A folder holding `shaders/`, or a `.zip`, in `shaderpacks/` shows in the Shader Packs screen next to Horizon packs.
Picking one translates every planned pass of each dimension folder (`java_pack.rs`) and runs them each frame
(`gfx/java_stages.rs`):

1. the targets the pack clears are cleared;
2. `begin*` and `prepare*` run, then Horizon's world enters `colortex0` and `depthtex0..2` on the side the flips give;
3. `deferred*` and `composite*` run in order, each reading the side its plan gives and writing the other;
4. `final` draws into a target that replaces the world's picture, where Horizon packs' `composite` output goes: the
   hand, menus and HUD are drawn over it.

When the pack has a `gbuffers_terrain` (or `textured_lit`, `textured`, `basic`, the program Iris falls back to), the
pack draws the terrain itself (`gfx/java_terrain.rs`): Horizon's world pass skips its own solid and translucent
terrain, then

- at step 2, after Horizon's sky, entities and the rest enter `colortex0` and depth, the pack's terrain program draws
  every visible section into its `RENDERTARGETS` (the side the next pass reads), tested against that depth; the
  depth is then copied into `depthtex0` and `depthtex1`;
- before the first `composite`, as Iris does, `gbuffers_water` (or the terrain program) draws the translucent
  layer with the pack's `blend.*` or vanilla's blending; the depth goes into `depthtex0` only.

The pack's vertex program reads Horizon's own terrain vertices (the arena pages, instanced or flat draws): a glue
function written before `main` decodes them into Iris's attributes (`vaPosition`/`gl_Vertex` relative to the camera,
`gl_Color` with vanilla's face shading undone, `gl_MultiTexCoord0/1`, the normal and `at_tangent` from the quad's
four corners, `mc_midTexCoord`, `at_midBlock`), and `mc_Entity.x` is the block's id from the pack's
`block.properties`, matched to the sprite's block by name. `gtexture`/`texture` is a 2D atlas built from Horizon's
sprite arrays (animated sprites copied in each frame), `lightmap` is Horizon's lightmap with Iris's texture matrix,
and fragments under alpha 0.1 are discarded (`alphaTestRef`). `RGBA16` targets are 16-bit unorm where the GPU can
render them (Chocapic13' packs its G-buffer into them; half floats broke it).

Occlusion culling works as in Horizon's world pass where the GPU has it: the pack's terrain program draws the
sections the last test saw, the boxes of the others are tested against that depth (in OpenGL's layout, in a pass of
their own since the pack's targets vary; on Metal the test reads the depth in the shader, as Horizon's does), and the
pack's program then draws the ones in sight, flat draws included.

When the pack has a `shadow` program (`shadow_cutout`, `shadow` or `shadow_solid` for the solid layer, which holds
cutout blocks; `shadow_water` or `shadow` for the translucent one), it draws a shadow map after `begin*`, as Iris's
`ShadowRenderer`:

- the light's matrices are Iris's `ShadowMatrices`: `shadowModelView` turns camera-relative positions toward the sun
  (the moon at night) with the pack's `sunPathRotation`, snapped to `shadowIntervalSize`; `shadowProjection` is
  orthographic, `shadowDistance` each way, between `shadowNearPlane` and `shadowFarPlane` (Iris's defaults when the
  pack sets none); the shadow programs get them as `gl_ModelViewMatrix` and `gl_ProjectionMatrix`;
- the renderer draws every section inside that box whose shadow can reach the camera's view (the box swept away
  from the light against the view's planes, as Iris's advanced shadow culling), from its own list (cave culling is
  off as in Iris, and no face is culled), solid then translucent, instanced draws only;
- `shadowtex0` holds all of it, `shadowtex1` the depth before translucent terrain; both are bound as depth textures
  for `sampler2DShadow` (linear comparison, Iris's hardware filtering) and as float copies for `sampler2D` reads;
- `shadowcolor0..7` are `shadowMapResolution` square in their `shadowcolor<N>Format`, cleared each frame to
  `shadowcolor<N>ClearColor` (white by default) unless `shadowcolor<N>Clear` is false.

A shadow program that doesn't translate or that wgpu refuses leaves the pack without its shadow map, with a warning:
the terrain programs still run and every `shadowtex` reads as far (nothing in shadow).

What makes Horizon look like OpenGL to the pack:

- **Layout.** The translator ends every vertex shader by flipping `gl_Position.y` and moving depth from `[-w, w]` to
  `[0, w]`, so the targets keep OpenGL's layout (row 0 at the bottom). The world is flipped going in, `final` coming out.
- **Depth.** Horizon's depth is reversed (1 near, 0 sky); the world goes in as `1 - depth`, and the matrices the pack
  gets (`gbufferProjection` and its inverse, previous frame too) are OpenGL's, rebuilt from Horizon's.
- **Formats.** Iris formats become renderable wgpu formats: 8-bit unorm ones and `RGB10_A2` stay, `*32F` stay (sampled
  unfiltered), integer formats become 32-bit, the rest (`R11F_G11F_B10F`, `*16F`, `*16`) 16-bit float, with the
  channel count of the name.
- **Samplers.** Metal allows 16 samplers per stage and Complementary binds 23 textures, so textures share four
  samplers by kind (`iris_linear`, `iris_nearest` for depth and 32-bit or integer targets, `iris_repeat` for
  `noisetex`, `iris_compare` for shadow lookups) instead of one each.
- **`isnan`/`isinf`** test the float's bits: Metal compiles with fast math, which folds `x != x` to false, and a NaN
  then stays forever in Complementary's TAA history (the picture turns black).
- **Uniforms.** Matrices, camera, view size, time, weather, fog and sky colour, eye state come from the frame;
  `eyeBrightness` is the block and sky light (× 16) of the camera entity's eye block, and `eyeBrightnessSmooth` follows
  it with the pack's `eyeBrightnessHalflife` (Iris's `SmoothedFloat`); `sunPosition`, `moonPosition`,
  `shadowLightPosition`, `sunAngle` and `shadowAngle` are `CelestialUniforms`' (the sky's `sun_angle` and `moon_angle`
  turned by Y -90, the pack's `sunPathRotation` about Z, then the angle about X); `noisetex` is a fixed 256² noise.
- **Custom uniforms.** `uniform.<type>.<name>` and `variable.<type>.<name>` in `shaders.properties` are evaluated
  each frame (`horizon-shaderpack`'s `uniforms.rs`: Iris's operators and priorities, its functions, `smooth` with
  `SmoothFloat`'s half-lives). A definition naming something Horizon doesn't provide (`biome`, `blindness`...) is left
  out with everything that uses it, as Iris leaves out the ones it can't resolve: the uniform stays 0.
- **The vanilla sky.** While the pack runs, `sunPathRotation` tilts Horizon's sun, moon and stars as Iris tilts
  vanilla's, `clouds = off | fast | fancy` replaces the cloud option, and `sun = false` / `moon = false` hide those.
  Complementary and Sildur's turn the vanilla clouds off: drawn anyway, they wrote depth the packs' light shafts and
  fog took for terrain.

Measured on the Mac mini (M4 Pro) with the packs' default options: BSL v10.1.8 and Complementary Reimagined r5.9.3
grade and light Horizon's world as their post-processing would. Sildur's Vibrant v2.02 draws only its sky: its
`deferred` reads the albedo `gbuffers_*` write to `colortex4` (not checked again since the terrain programs run).
Without the other `gbuffers_*`, no pack has its own sky yet, and only terrain casts shadows; Distant Horizons' and
Voxy's depth read as far. Horizon's world goes into the target the pack's `gbuffers_terrain` (or the program Iris falls
back to) draws to first: `colortex0` for most packs, `colortex1` for MakeUp. Not supported yet: images and compute
(`.csh`; a pack that needs them doesn't load), mipmapped targets (`colortexNMipmapEnabled` is read but ignored), `.dat`
and resource-location custom textures (PNG files in the pack only), `shadowcomp` passes, `shadowMapFov` (a perspective
shadow map), `shadow.culling` and `shadowDistanceRenderMul` (the box above is the only bound). With the pack's terrain:
entities, the sky, the hand, weather and clouds are still Horizon's (in that first target only), block ids come from
sprite names rather than block states, distant (LOD) terrain doesn't repeat its textures, and only the overworld has
been checked.

### Compatibility

Tested on 2026-10-22 on the Mac mini (M4 Pro, Metal) with each pack's default options, the 26.3 builds from Modrinth or
the authors' pages (dev-only test inputs, kept outside the repository). "Loads" means every pass the pack enables for
the overworld, Nether and End is preprocessed, translated and validated, as the game does when the pack is picked
(`java_pack::tests::real_packs_load` with `HORIZON_TEST_SHADERPACKS`); "drawn" means it was also picked in the game
and screenshotted. Full-screen passes, `gbuffers_terrain`, `gbuffers_water` and the terrain's shadow map run
(above); no pack has its own entities or sky yet.

| Pack | Version | Result | First thing that stops it |
| --- | --- | --- | --- |
| BSL | v10.1.8 | drawn: 26 passes and its terrain and water programs | |
| Complementary Reimagined | r5.9.3 | drawn: 27 passes | |
| Complementary Unbound | r5.8 | loads: 27 passes | |
| Sildur's Vibrant Extreme | 2.02 | drawn: 18 passes, only its sky (above) | |
| Steadfast | 0.8.0 | drawn: 3 passes | |
| Chocapic13' | V9 High | drawn: 16 passes and its terrain and water programs | |
| MakeUp Ultra Fast | 9.6 | drawn: 6 passes and its terrain and water programs | |
| Photon | v1.4 | loads: 27 passes and its terrain and water programs; draws black | its sky and light come from a compute pass and `.dat` look-up tables, not loaded yet |
| Bliss | 2.1.2 | fails | `world0/composite1.fsh`: a vector component passed to an `inout` parameter (WGSL can't point at one) |
| Bloop | 1.8.0-Alpha-3 | fails | `world0/prepare1.fsh`: `mat2(vec4)` and other constructors naga's GLSL front end refuses |
| Noble | v1.9.9 | fails (Apple and NVIDIA) | `world0/deferred4.vsh`: a `vec3[9][9]` varying (nested arrays between stages) |
| Rethinking Voxels | r0.1-beta9 | fails | `world0/prepare.fsh`: `1 << 31` overflows naga's constant folding; it also needs images |

What the translator does for these packs, as Iris (JCPP and its GLSL transformer) does:

- **`#if` reads floats as JCPP does**: a float in a condition, written there or through a macro, counts as its
  integer part, and an empty operand as 0. The same holds in `shaders.properties`, where Steadfast's
  `#elif ATMOSPHERE_FOG_STRENGTH_NOON == 0.5` stopped the pack (`UnexpectedToken(Float(0.5))`).
- **A later `#define` replaces an earlier one** instead of failing (Bloop's `DefineRedefined`), and an unterminated
  `#if` runs to the end of the file.
- **`shaders.properties`** keeps its empty lines and backslashes through the preprocessor, as Iris's does, so a
  continued line doesn't swallow the next one.
- **16-bit and 8-bit types** (`float16_t`, `u16vec2`, `packFloat2x16`...), which Noble uses when the GPU reports
  NVIDIA (`NV_gpu_shader5`), become their 32-bit types: WGSL has none.
- **What naga refuses**: `%` with a scalar and a vector operand goes through overloads, a `const` naga can't fold
  (`mix`, `?:`, `smoothstep`) becomes a global set at the start of `main`, `sqrt` of a negative constant is
  evaluated at run time, and a fragment output narrower than four channels (`out vec3`, Steadfast's `final`) is
  written through a `vec4` with alpha 1 as OpenGL fills it.
- **What GL takes and naga doesn't** (Photon, Chocapic13', MakeUp Ultra Fast, Bliss): `texture2D` and its kin without a
  level outside fragment shaders sample level 0 (`textureLod`); a struct passed between stages travels as one varying
  per field (matrices by column, arrays by element); `textureGather` reads the four texels with `texelFetch`; a
  `bool` given to `vec3(..)` and other numeric constructors becomes `cond ? 1.0 : 0.0`; `const in` parameters lose
  the `const`; an array field of a struct that never reaches a buffer gets its natural stride (naga lays every struct
  out as std140, so `Spline(.., float[6](..))` didn't match); `modf` into a vector component assigns it instead of
  passing a pointer.
- **Every translated program parses again as WGSL** in `real_packs_load`: wgpu is given the text, and naga's WGSL
  writer can emit what its WGSL parser refuses (`&v.y`).

### Order to implement

1. **Translator crate** (`horizon-shaderpack`): the preprocessor (includes, option `#define`s with the player's
   values, Iris macros) and a syntax-tree legaliser covering every rewrite above plus the three fixes in the table;
   a test running it over the user's own copies of the three packs when they're present (never committed), and
   small hand-written GLSL fixtures in the repo.
2. **`shaders.properties`** and option screens: the option list, sliders, profiles, `program.*.enabled`, blend modes.
3. **Full-screen programs**: `deferred*`, `composite*`, `final` with `colortex0..15` (formats from the pack),
   `depthtex0..2`, `noisetex` and the uniforms (most are already computed for the world shaders). This alone runs
   Sildur's and BSL post-processing over Horizon's own world.
4. **`gbuffers_terrain`, `gbuffers_water`** (done, above), **`gbuffers_sky*`**: Horizon's terrain vertices decoded
   into Iris's attributes (`mc_Entity` from `block.properties`, `mc_midTexCoord`, `at_tangent`, lightmap), a second
   set of terrain pipelines writing the pack's render targets.
5. **Entities, block entities, hand, weather, clouds, particles** `gbuffers_*`.
6. **Shadow pass** (terrain done, above): entities and block entities from the sun into `shadowtex0/1` and
   `shadowcolor0/1`, culled from the light's view, with the pack's distortion; `shadowcomp`.
7. **The rest**: custom textures, compute programs and images, per-dimension
   folders, `entity.properties`/`item.properties`.
