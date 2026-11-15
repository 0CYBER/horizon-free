# Horizon shader packs

Horizon has its own shader packs: WGSL written against stages of Horizon's renderer, checked before they run. This page
is the guide for pack authors. The design and the pack format are in [api/shaders.md](api/shaders.md); every stage's
signature, input, resource and limit is in the generated [api/shaders-reference.md](api/shaders-reference.md).

Shader packs in the Iris/OptiFine format are a separate feature with its own loader; this page is only about
Horizon's own format.

| Part | Where | State |
|---|---|---|
| Format, stages, check | `crates/horizon-shader-api` | API 0.1, experimental |
| Authoring tool | `horizon-shader` (same crate) | `new`, `check`, `pack` |
| Templates | `crates/horizon-shader-api/templates/` | post, sky, material |
| Example pack | `crates/horizon-shader-api/packs/golden_hour` | a warm sky and two post passes |
| In the game: every stage | Options > Video Settings > Shaders > Shader Packs | sky, material, post passes, hot reload |
| The registry and the launcher | `shader` kind ([api/mod-index.md](api/mod-index.md)); the Mods page's Shader packs tab | checked, then installed into `shaderpacks/` |

## Quick start

```sh
cargo install --path crates/horizon-shader-api      # or: cargo run -p horizon-shader-api --bin horizon-shader -- <command>
horizon-shader new my_pack --template post          # also: --template sky, --template material; --name "My Pack"
horizon-shader check my_pack                        # what the game, the launcher and the registry will check
horizon-shader pack my_pack                         # check, then write my_pack-0.1.0.hshader beside the folder
```

`check` prints each stage with its cost, and the pack's SHA-256:

```
golden_hour 1.0.0 (shader API 0.1): ok
  sky       Warm horizon                     99 expressions, 0 texture reads
  post      Glow                            192 expressions, 3 texture reads
  post      Colour grade                    201 expressions, 2 texture reads
  sha256 b5e0d98c...
```

or every problem, at its file, line and column:

```
my_pack 0.1.0 (shader API 0.1): FAILED
  post.wgsl:9:1: expected `;`, found "}"
```

`--json` prints the same for tools (`ok`, `id`, `version`, `api`, `sha256`, `stages`, `problems`).

## Trying a pack in the game

Put the pack folder (or its `.hshader`) in `shaderpacks/` beside `options.txt` (Open Pack Folder opens it) and pick
it on Options > Video Settings > Shaders > Shader Packs..., which lists Horizon's packs, marked (Horizon), beside packs in the
`shaders/` format; one runs at a time, and clicking it again turns it off. `HORIZON_SHADER_PACK=<its folder or file
name>` runs a pack whatever the page says, for development. The game checks the pack first, and refuses one that
fails, with its problems in the chat. While it runs, the game looks at the pack twice a second: save a file
and the pack is checked and rebuilt in place ("Shader pack ... reloaded"); if the new version fails, the chat says
where and the last good version keeps drawing. All three stages run.

A picked pack's options are pages of the same screen, under the pack's name in its page list: its screens, with the
names the pack gives them, its profiles, and Reset. The main screen and those it links to are in the list (when
there are more than it has room for, they collapse into one `‹ Name ›` row that the arrows and a controller's
bumpers step through); deeper screens open from their links, and Escape goes back. A click, Enter or Right picks an
option's next value, a Shift click or Left the one before, and the options the pack lists in
`sliders` are sliders. Pointing at one shows what the pack says of it (`option.NAME.comment`); the search finds an
option on any screen. Undo puts back the pick, the Shadow Cache, the Horizon packs' options and the options of the
`shaders/` pack that was picked when the screen opened; another `shaders/` pack keeps what was saved for it (picking
away from it saves its changes, as Apply does). A `shaders/` pack's
options are saved to `shaderpacks/<pack>.txt` (`NAME=value` lines), and the pack loads again with them, with Apply
(shown while they differ from the saved ones) or on leaving the screen; a Horizon pack's apply at once.

## Testing a pack frame by frame

A pack that keeps something from frame to frame (`history`, eased values) draws a little differently in every run,
as frames come when they come. With `HORIZON_STILL=1` and the world frozen the scene holds still
([agent-control.md](agent-control.md#repeatable-screenshots)), and

```sh
horizon-ctl screenshot-sequence 64 50 frame64.png
```

starts the pack over as if just loaded (black `history`, eased values from the start) and draws 64 frames 50 ms
apart on its clock: frame n has `frame_index` n, `time` (n - 1) × 0.05 and `delta_time` 0.05 (0 on frame 1). The
64th is written to `frame64.png` (and `frame64-hdr.png` with HDR output on) before the reply. STEP_MS may be a
decimal: `16.666667` steps a 60 fps clock. The same build, GPU
and driver write the same bytes every run, so two versions of a pack compare by a pixel diff. `screenshot-sequence
cancel` stops one.

## A pack

A folder (or the same files zipped as a `.hshader`) with `horizon-shaders.toml` and WGSL files:

```toml
schema = 1
id = "my_pack"
name = "My Pack"
version = "0.1.0"
api = "0.1"

[[post]]
file = "post.wgsl"

[[options]]
id = "saturation"
name = "Saturation"
type = "float"
default = 1.15
min = 0.0
max = 2.0
```

Every key is in [api/shaders.md](api/shaders.md#the-pack-format-schema-1).

## Stages

A stage is one function your file defines. Horizon puts its prelude (the structs, `frame`, the stage's resources and
helpers) before your file and calls the function from its own pass, so the world is still culled, batched and meshed
by Horizon: a pack changes how things look, not how they are drawn.

| Stage | Your function | What it does |
| --- | --- | --- |
| `[sky]` | `fn sky(input: SkyInput) -> vec4<f32>` | the sky's colour along each view ray, opaque (alpha is ignored), drawn where vanilla draws its sky dome (the Overworld's sky; not the Nether's or the End's); the sun, moon, stars and clouds are drawn over it |
| `[material]` | `fn material(input: MaterialInput) -> Surface` | the lit colour of each point of a block's surface, before the alpha test and fog: every terrain section and distant tile, solid, cutout and translucent (not entities, block entities or particles) |
| `[weather]` | `fn weather(input: WeatherInput) -> vec4<f32>` | the colour of each point of a rain or snow streak, blended over the world with its alpha; Horizon's light and fog aren't applied, so the pack applies its own |
| `[[post]]` | `fn post(input: PostInput) -> vec4<f32>` | a full-screen pass over the world's picture (up to 8, in order), under the hand and HUD. Return `input.color.a` as your alpha: HDR output reads it to find light sources |

A pack has at least one stage; a stage it leaves out keeps vanilla's look. Each input says what vanilla would draw
(`input.color` in the sky and post stages; `albedo * tint * vec4(lightmap, 1.0)` for a material), so a pack can start
from vanilla and change only what it means to.

Every stage can read `frame`: the camera's matrices and position, the sun and moon directions, the time of day,
vanilla's sky and fog colours, rain and thunder, fog distances, the light at the camera, effects and the dimension.
Post passes also read `scene_color`, `scene_depth`, `previous` (the last pass's output), two samplers and the helpers
`depth_at(uv)` and `view_position(uv, depth)`.

Rules the check enforces: your files declare no bindings (`@group`/`@binding`), no entry points and no names starting
with `horizon_`. Code shared by several stages goes in files listed in `shared`, put before every stage.

## Sprite tags

The material stage can tell block sprites apart through tags the manifest declares, and so can a traced ray's hit
(`RayHit.tags`, [ray tracing](#ray-tracing)): a pack with tags has a `[material]` stage or `[ray_tracing]`. Each
`[[tags]]` is a bit of `input.tags`, in the order they appear (32 at most); its `sprites` are sprite names, the namespace `minecraft` by
default, each with at most one `*`:

```toml
[[tags]]
id = "glass"                                      # bit 0: (input.tags & 1u) != 0u
sprites = ["block/glass", "block/*_stained_glass"]
```

Terrain carries its sprite, not its block, so a sprite drawn by several blocks gets the tag for all of them.

## Material ids

Tags answer "is it one of these"; a pack that treats hundreds of kinds of surface differently numbers them
instead. Each sprite takes the `id` of the first `[[materials]]` entry that names it, 0 without one, as
`input.material` (and a traced ray's `RayHit.material`). Patterns may hold any number of `*`:

```toml
[[materials]]
id = 10
sprites = ["block/*_leaves"]

[shadow]
solid_materials = [10]   # these cast whole shadows: their cutouts are ignored in the shadow map
```

## Textures

Each `[[textures]]` entry is a PNG in the pack that every stage reads under the name of its `id`, with a sampler named
`<id>_sampler`. Eight at most, without mipmaps; `filter` is `linear` (the default) or `nearest`, `wrap` is `repeat` (the
default) or `clamp`:

```toml
[[textures]]
id = "noise"
file = "noise.png"
```

```wgsl
let n = textureSample(noise, noise_sampler, input.uv);
```

## Coloured block light

Each `[[lights]]` entry gives the blocks it names a colour. The material stage reads the colour at a point with
`block_light_color(input.position)`: rgb the colours of the sources nearby, averaged by how near each is, and a how
strongly they reach it (0 where none does). Block light keeps its own brightness, so walls still stop it: this is only its
colour, and only what a pack does with it shows.

```toml
[[lights]]
color = [0.3, 0.7, 1.0]
blocks = ["soul_torch", "soul_wall_torch", "soul_lantern"]
```

```wgsl
let tint = block_light_color(input.position);
let block = mix(vec3(1.0), tint.rgb, min(tint.a, 1.0)) * input.light.x;
```

It covers 128 blocks around the camera, moved in steps of 16, and is rebuilt on the CPU when the camera steps or a source
changes, from at most 2048 sources, the nearest first.

## Ray tracing

A pack of API 0.2 can trace rays through the world from its post passes and its material stage. It asks with an empty
`[ray_tracing]` table (the pack needs a `[[post]]` pass or a `[material]` stage), and its post stage gets the trace
functions, as does its material stage where its files call one ([the reference](api/shaders-reference-0.2.md#ray-tracing) has
`RayHit`'s fields):

```toml
api = "0.2"

[ray_tracing]

[[post]]
file = "post.wgsl"
```

```wgsl
let position = view_position(input.uv, input.depth);
// A little toward the camera, off the surface the point is on.
if frame.has_ray_tracing == 1u && trace_any(position * 0.99, frame.sun_direction, 512.0) {
    // Terrain stands between the point and the sun.
}
let hit = trace(position * 0.99, direction, 64.0);   // hit.distance < 0.0: nothing there
```

`trace` returns the nearest solid surface along a ray, with what the material stage would get for that point (`albedo`,
`tint`, `light`, `uv`, `material`, `tags`, `emission`, `fluid`, `translucent`, `tangent`) and its `position`, `normal`
and `distance`; `trace_any` only says whether something is in the way, which is all a shadow needs. Origins are camera-relative, as `view_position` gives them, and
rays pass through the clear texels of leaves and plants, and through water, stained glass and ice, which are in a
[layer of their own](#layers). Horizon keeps the scene; the look is the pack's: Horizon
adds no shadows, bounce light or denoising of its own. The pack never sees the acceleration structure or a ray query,
and the check refuses a pack that names one itself.

| | |
| --- | --- |
| In the scene today | Terrain within 8 chunks of the camera, in two layers: solid terrain (cut-outs and lava included), which `trace` reads, and translucent terrain (water, stained glass, ice), which [`trace_layers`](#layers) reads; `frame.ray_distance` is how far it surely reaches |
| Not in it yet | Entities, block entities, the hand, distant terrain tiles |
| Turning it on | Pick the pack and start the game with it, on a GPU and graphics API with hardware ray queries: the device needs them from its creation, so in a game started without them a pack picked while it runs traces from the next start, and the shader pack screen says so. `HORIZON_RAY_TRACING=1` asks for ray queries and keeps the scene with any pack or none; any other value of it turns ray tracing off |
| Without it | The pack loads and runs as it is: `frame.has_ray_tracing` is 0, `trace` misses and `trace_any` is false. Branch on `has_ray_tracing` and keep a look that works without rays. It is also 0 in a pack none of whose stages calls a trace function |

A hit also reads the resource packs' LabPBR maps, as a drawn surface does: `hit_normal_map(hit)` and
`hit_specular_map(hit)` give its sprite's texel at full size, with the same defaults as `block_normal_map` and
`block_specular_map` where there are no maps (and on a miss), and `block_pbr_maps()` says whether there are, in a post
pass too (there it is false while the game doesn't trace, when no hit reads them). `hit.tangent` turns the normal map into world axes as `input.tangent` does. A reflection shaded with the
roughness the same block has on screen:

```wgsl
let hit = trace(position * 0.99, direction, 64.0);
let smoothness = select(0.5, hit_specular_map(hit).r, block_pbr_maps());   // LabPBR's perceptual smoothness
let roughness = (1.0 - smoothness) * (1.0 - smoothness);
let shine = pow(max(dot(reflect(direction, hit.normal), frame.sun_direction), 0.0), 2.0 / max(roughness * roughness, 1e-4));
let color = hit.albedo.rgb * hit.tint.rgb * (hit.light.y + hit.emission) + vec3<f32>(shine * hit.light.y);
```

The scene trails the world by a frame or two where chunks load or blocks change, and is empty for the first frames in
a world. The smallest working pack is `crates/horizon-shader-api/packs/traced_shadows`: the sun's shadows, one ray a
pixel.

### Layers

The scene has two layers, and a ray reads the ones it names. `trace` and `trace_any` read solid terrain alone;
`trace_layers(origin, direction, max_distance, layers)` and `trace_any_layers(...)` take the layers, `RAY_SOLID`,
`RAY_TRANSLUCENT` or both (`RAY_SOLID | RAY_TRANSLUCENT`), and are otherwise the same.

| Layer | Holds | A ray stops |
| --- | --- | --- |
| `RAY_SOLID` | What the solid pass draws: opaque blocks, cut-outs (leaves, plants), lava | At an opaque texel, or a cut-out's texel of alpha 0.5 or more: the solid pass's alpha test |
| `RAY_TRANSLUCENT` | What the translucent pass draws: water, glass and panes, ice, slime and honey blocks; any block whose texture has partly see-through texels | At any texel the translucent pass draws, however faint: only a clear one (alpha below 0.01) lets it through |

A hit in the translucent layer has `translucent` true, `fluid` 1 on water, and its `albedo.a` is the texture's alpha
there, so a pack decides what the surface does to the ray: a ray over both layers stops at the nearest surface of
either, and goes on when the pack traces again from just behind the hit. A face stops a ray from either side. The
sun's light at a point, cut off by solid terrain and tinted by the first glass or water on the way:

```wgsl
var origin = position * 0.99;                                          // off the surface the point is on
var light = vec3<f32>(1.0);
var layers = RAY_SOLID | RAY_TRANSLUCENT;
for (var i = 0; i < 2; i++) {
    let hit = trace_layers(origin, frame.sun_direction, 512.0, layers);
    if hit.distance < 0.0 { break; }                                   // nothing more in the way
    if !hit.translucent { light = vec3<f32>(0.0); break; }             // solid terrain: in shadow
    light = mix(light, hit.albedo.rgb * hit.tint.rgb, hit.albedo.a);   // glass or water: tinted
    origin = hit.position + frame.sun_direction * 0.01;                // on from just behind it,
    layers = RAY_SOLID;                                                // through any other glass
}
```

The translucent layer is the terrain's translucent quads as they are meshed: still and flowing water alike. A picture
made with `trace` and `trace_any` alone is the same with the layer as without it.

**In the material stage.** Translucent terrain (water, stained glass, ice) is blended where `material` shades it and
leaves no surface for a post pass to light, so that is where its rays go. `input.position` and `input.normal` are
camera-relative and in world axes, so a ray starts from them as they are:

```wgsl
if input.translucent && frame.has_ray_tracing == 1u {
    let hit = trace(input.position, reflect(normalize(input.position), input.normal), 64.0);
}
```

- **It costs more per ray.** `material` runs for every terrain fragment drawn, hidden ones included where the depth
  test hasn't rejected them yet, so a ray there costs more than the same ray in a post pass, which runs once a pixel.
  Light solid terrain in a post pass (`view_position`, `surface_at`) and keep the material stage's rays for translucent
  surfaces (`input.translucent`).
- **Only `material` itself traces.** `displacement` and the `[vertex]` file run in vertex shaders: the check refuses a
  pack whose `displacement` or `vertex` reaches a trace function.
- **`trace` passes through water.** A ray from a water surface passes through other water: right for the shore's
  reflection, wrong for what lies behind a waterfall. Name the layer to find it: `trace_layers(..., RAY_SOLID |
  RAY_TRANSLUCENT)`, from a point off the surface (`input.position + input.normal * 0.01`), which is in that layer
  itself.
- **Only a stage that calls them pays.** A material stage whose files name no trace function is built as without
  `[ray_tracing]`; one that does is built twice, the terrain drawing with the rays in frames that have the scene.

`crates/horizon-shader-api/packs/traced_water` is the smallest pack that traces from `material`: water that mixes in
what a reflected ray hits.

## Compute passes and kept resources

Shader API 0.2 has the keys for storage a pack keeps between frames and for compute passes: `[[resource]]` (buffers,
and textures with this frame's and last frame's versions), `[kept] reach`, `[[before_world]]`, `[[post]]` entries
with `compute = true`, and an option's `resets`. A compute file defines `fn compute(input: ComputeInput)`.
`horizon-shader check` takes them now, but **they run in a later Horizon**: this game refuses a pack that declares a
resource or a compute pass. The keys, their limits and the WGSL each pass sees are in the
[reference](api/shaders-reference-0.2.md#kept-resources-and-compute-passes).

Two parts come in later steps, not with these keys: a compute or `[[before_world]]` file doesn't get `trace` or the
other ray functions under `[ray_tracing]`, and `[material]` can't read kept resources.

## Options

A picked pack with options gets a Shader Options... button at the top of the Shader Packs page. Float and int options
are sliders that take the nearest `step` (an int's whole numbers), and the arrow keys move a focused one a step at a
time; bool and choice options are buttons, where a click picks the next value and a right click the one before.
Shift-click puts either back to its default, and Reset all of them; hovering one shows its `description`. Each change
is saved to `shaderpacks/<pack>.txt` (`id=value` lines, values at their defaults left out), a slider's when it is
let go, and the pack reloads with it within half a second.

The page shows every option two a row, unless the pack lays them out in `[[pages]]`: it opens on the first page, and an
item `"page:<id>"` is a button opening another, so options can be grouped into categories. `""` leaves a slot empty, and
`columns = 1` gives a page full-width buttons:

```toml
[[pages]]
id = "main"
name = "My Pack"
items = ["page:lighting", "page:water"]

[[pages]]
id = "lighting"
name = "Lighting"
description = "Shadows and the light of blocks."   # the tooltip of the button opening it
items = ["shadow_softness", "block_light"]
```

Every page has to be reached from the first and every option has to be on a page, or the check refuses the pack.

Each `[[options]]` entry is a WGSL `override` of the same name, set when the game builds the pipeline: no text
substitution, no recompiling a different program per value.

```wgsl
override saturation: f32 = 1.15;   // type = "float"; bool → bool, int → i32, choice → u32 (the value's index)
```

The game shows the options on the pack's settings screen. Branches on an option are on a constant, so the GPU
skips the side that isn't taken.

The shadow map is most of what a pack with `[shadow]` costs: each frame the terrain in the light's square is drawn again.
Options can size it, so players pick their own quality. While `option` is off there is no map and no shadow pass
(`frame.has_shadow` reads 0); `resolution_option` and `distance_option` name an `int` option, or a `choice` whose values
are numbers, whose value replaces `resolution` or `distance`, checked against their ranges. Changing one rebuilds the map
at the new size. These two need no `override` in your WGSL: the frame has the map's size.

```toml
[shadow]
option = "shadows"                        # a bool: no shadow pass while it is off
resolution_option = "shadow_resolution"

[[options]]
id = "shadow_resolution"
name = "Shadow resolution"
type = "choice"
values = ["1024", "2048", "4096"]
default = "2048"
```

## What the check does

The game runs the same check when it loads a pack (a pack that fails is refused with the message, and the last good
one keeps drawing), the launcher before it installs one, and the registry's CI before a pack is listed:

1. The manifest and the package: known keys, valid ids, every file used, sizes.
2. Each stage built as the renderer builds it and validated by naga.
3. The stage function's signature, and the rules above.
4. Each `override` matches its option's type (or has a default); each option is used (those sizing the shadow map
   may be by `[shadow]` alone).
5. Budgets: 20 000 expressions and 64 texture reads per stage.
6. Every entry point translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL), with the options at their
   defaults, minimums and maximums.

`[budget] gpu_ms` is the GPU time your pack aims to add to a frame. To compare, open F3 in the game: while it is
open, each pass is timed on the GPU and the overlay shows the pack's total and its costliest passes, by their `name`
(the median of the last 15 timed frames). Timing waits for nothing, so it does not slow the game. In a timed frame the sky
stage draws in a pass of its own (it shares the world's pass otherwise), so its time includes clearing the picture. The
material stage runs inside terrain's own draws, so it has no time of its own: compare the frame's GPU time with and
without the pack.

## Next

An overlay showing what each pass draws (F3 already shows what each costs), and stages for entities, block entities
and particles (the material stage covers terrain only).
