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
| Horizon shader packs: `composite` stage, uniforms, hot reload, Shader Packs screen | first slice planned |
| Horizon shader packs: `deferred`, `gbuffer`, `shadow` stages, custom textures | planned |
| Java shader packs: preprocessing + GLSL parse coverage | spike done: 450 of 470 programs reach valid WGSL (below) |
| Java shader packs: Iris pipeline emulation | planned |

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
6. **Menu grade** (`post.rs`, `post.wgsl`): the Horizon menus' frosted backdrop.
7. **HUD** (`hud.rs`, `hud.wgsl`): GUI, text, items in slots.

All world shaders share one uniform block, `Globals` in `gfx.rs` (`struct Globals` in `terrain.wgsl`):
view-projection, inverse view, fog colour and distances, sky colour, sun direction, sunrise colour, game time.
The lightmap, fog and sky colours are computed on the CPU (`environment.rs`, `fog.rs`) as vanilla does.

## Horizon shader packs

### Where packs live and how one is picked

- Packs go in `shaderpacks/` beside `options.txt` in the game folder (the same folder Iris uses, so one folder
  holds both kinds). A pack is a folder or a `.zip`. A pack with a `pack.toml` at its root is a Horizon pack; a pack
  with a `shaders/` folder is a Java pack.
- **Options → Video Settings → Shader Packs...** (Classic screens, laid out like Iris's: the packs on the left with
  "(off)" first, Apply and Done, *Open Shader Pack Folder*, *Shader Pack Settings...*) and a **Shader Packs** row in
  the Horizon menus' Video section open the same list.
- The choice is kept in `horizon-graphics.txt` (`shaderPack:<folder or zip name>`, empty = off). A pack's option
  values are kept in `shaderpacks/<pack>.txt`, as Iris does.
- A pack that fails to load or compile leaves the vanilla picture on and shows why, in red, on its row (and in the
  log). Nothing a pack does can stop the game from starting.
- Benchmarks (`--benchmark`) and the parity harnesses never load a pack.

### Layout

```
my-pack/
  pack.toml
  shaders/
    composite.wgsl
    bloom.wgsl
  textures/
    noise.png          # planned
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
    sun: vec4<f32>,                 // xyz: direction to the sun (view-independent, world space), w: sun angle (vanilla's)
    moon: vec4<f32>,                // xyz: direction to the moon, w: moon phase (0..7)
    sky_color: vec4<f32>,           // vanilla's sky colour this frame
    fog_color: vec4<f32>,           // vanilla's fog colour this frame (what the world pass clears to)
    fog: vec4<f32>,                 // environmental fog start, end; sky fog end; cloud fog end (blocks)
    weather: vec4<f32>,             // rain level, thunder level, 0, 0 (smoothed as vanilla does)
    camera_state: vec4<f32>,        // submerged in: 0 air, 1 water, 2 lava, 3 powder snow; night vision; darkness; blindness
    view_params: vec4<f32>,         // near, far (Camera.depthFar), render distance (blocks), vertical FOV (radians)
    dimension: vec4<f32>,           // 0 overworld, -1 nether, 1 end; has sky light; 0, 0
}
```

Prelude helpers: `linear_depth(depth) -> f32` (blocks from the eye), `view_position(uv, depth) -> vec3`,
`world_offset(uv, depth) -> vec3` (camera-relative world position), `is_sky(depth) -> bool`, `luminance(rgb)`,
`srgb_to_linear` / `linear_to_srgb`. They hide reversed depth, so a pack doesn't depend on it.

The uniform layout only grows: a new field goes at the end and the `api` number goes up; a pack that declares an
older `api` still loads.

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
| Distant Horizons, Voxy hooks, `DH_*` | not planned | Horizon has no LOD terrain yet |

Order: preprocessor and legaliser until the three test packs parse; then `composite`/`final`/`deferred` with the
uniforms; then `gbuffers_terrain`, `gbuffers_water` and `gbuffers_skybasic`/`skytextured` (the packs' look comes
from these); then entities, hand, weather, clouds; then `shadow`; then custom uniforms, custom textures, compute.

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

So the GLSL side is feasible: naga's front end plus a legaliser reaches 96% on the three packs, and every remaining
failure has a mechanical fix. Valid WGSL isn't a working pack yet: the output still has to be bound to Horizon's
pipeline (render targets, the uniforms' values, vertex attributes for `gbuffers_*`), which is the emulation work.
The spike also showed what the real translator needs that a token rewriter does badly: scopes (a local named like a
uniform), types (which `texture` call returns a float) and declarations split across `#if`s, so it should rewrite a
GLSL syntax tree after preprocessing, not tokens.

### Order to implement

1. **Translator crate** (`horizon-shaderpack`): the preprocessor (includes, option `#define`s with the player's
   values, Iris macros) and a syntax-tree legaliser covering every rewrite above plus the three fixes in the table;
   a test running it over the user's own copies of the three packs when they're present (never committed), and
   small hand-written GLSL fixtures in the repo.
2. **`shaders.properties`** and option screens: the option list, sliders, profiles, `program.*.enabled`, blend modes.
3. **Full-screen programs**: `deferred*`, `composite*`, `final` with `colortex0..15` (formats from the pack),
   `depthtex0..2`, `noisetex` and the uniforms (most are already computed for the world shaders). This alone runs
   Sildur's and BSL post-processing over Horizon's own world.
4. **`gbuffers_terrain`, `gbuffers_water`, `gbuffers_sky*`**: Horizon's terrain vertices extended with Iris's
   attributes (`mc_Entity` from `block.properties`, `mc_midTexCoord`, `at_tangent`, lightmap), a second set of
   terrain pipelines writing the pack's render targets.
5. **Entities, block entities, hand, weather, clouds, particles** `gbuffers_*`.
6. **Shadow pass**: terrain and entities from the sun into `shadowtex0/1` and `shadowcolor0/1`, culled from the
   light's view, with the pack's distortion.
7. **The rest**: custom uniforms (`uniform.*` expressions), custom textures, compute programs and images, per-dimension
   folders, `entity.properties`/`item.properties`.
