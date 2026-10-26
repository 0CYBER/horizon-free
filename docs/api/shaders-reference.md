# Shader API reference (0.1)

Generated from `crates/horizon-shader-api` (`HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test reference`
rewrites it; the test fails when it drifts). The API's design and pack format: [shaders.md](shaders.md).

A stage is a function your WGSL defines, with the signature below; Horizon puts the prelude (these structs, the
resources and helpers) before your files and calls the function from its own pass. Your files declare no
bindings and no entry points, and no names starting with `horizon_`.

## Stages

| Stage | In `horizon-shaders.toml` | Your function |
| --- | --- | --- |
| sky | `[sky]` | `fn sky(input: SkyInput) -> vec4<f32>` |
| material | `[material]` | `fn material(input: MaterialInput) -> Surface` |
| post | `[[post]]` | `fn post(input: PostInput) -> vec4<f32>` |

## sky

`fn sky(input: SkyInput) -> vec4<f32>`

### `SkyInput`

One direction of the sky, for `fn sky(input: SkyInput) -> vec4<f32>`. The sun, moon, stars and clouds are drawn over the result.

| Field | Type | Meaning |
| --- | --- | --- |
| `direction` | `vec3<f32>` | Unit view ray, camera-relative world axes. |
| `color` | `vec4<f32>` | Vanilla's sky colour along the ray (sky, fog and sunrise). |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |

## material

`fn material(input: MaterialInput) -> Surface`

### `MaterialInput`

One point of a block's surface, for `fn material(input: MaterialInput) -> Surface`.

| Field | Type | Meaning |
| --- | --- | --- |
| `albedo` | `vec4<f32>` | The block texture's colour here (alpha 0..1). |
| `tint` | `vec4<f32>` | The vertex colour: biome tint, face shade and ambient occlusion. |
| `light` | `vec2<f32>` | Block and sky light, 0..1, smoothed over the face. |
| `lightmap` | `vec3<f32>` | Vanilla's light colour for that light this frame. |
| `position` | `vec3<f32>` | The point, camera-relative, in blocks. |
| `uv` | `vec2<f32>` | The texture coordinate inside the block atlas. |
| `translucent` | `bool` | Drawn in the translucent pass (water, stained glass, ice). |

### `Surface`

What `material` returns. Vanilla's own is `albedo * tint * vec4(lightmap, 1.0)`; Horizon then applies the alpha test (cutout and translucent) and fog.

| Field | Type | Meaning |
| --- | --- | --- |
| `color` | `vec4<f32>` | The lit colour, 0..1, with its alpha. |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |

## post

`fn post(input: PostInput) -> vec4<f32>`

### `PostInput`

One pixel of the picture, for `fn post(input: PostInput) -> vec4<f32>`.

| Field | Type | Meaning |
| --- | --- | --- |
| `uv` | `vec2<f32>` | Where the pixel is, 0..1 from the top left. |
| `pixel` | `vec2<f32>` | The pixel's centre, in pixels from the top left of what the pass draws (the world's size; the window's for the last pass). |
| `color` | `vec4<f32>` | The previous pass's colour here (the world's picture for the first pass). |
| `depth` | `f32` | The world's depth here, reversed: 0 is the sky. |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |
| 1 | `var scene_color: texture_2d<f32>` | The world's picture before any pass. |
| 2 | `var scene_depth: texture_depth_2d` | The world's depth, reversed (1 near, 0 far and sky). |
| 3 | `var previous: texture_2d<f32>` | The previous pass's output (`scene_color` for the first pass). |
| 4 | `var linear_sampler: sampler` | Bilinear filtering, clamped to the edges. |
| 5 | `var nearest_sampler: sampler` | Nearest texel, clamped to the edges. |

Helpers:

- `fn depth_at(uv: vec2<f32>) -> f32`: the world's depth at `uv`.
- `fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32>`: the camera-relative position, in blocks, of
  the point at `uv` and `depth`.

## The frame

### `Frame`

The frame, as `frame` in every stage. Positions and directions are in world axes relative to the camera (x east, y up, z south), so they stay precise far from the world's origin; depth is reversed (1 near, 0 at the far plane and in the sky).

| Field | Type | Offset | Meaning |
| --- | --- | --- | --- |
| `view` | `mat4x4<f32>` | 0 | Camera-relative world to view space: the camera's rotation. |
| `projection` | `mat4x4<f32>` | 64 | View to clip space (reversed depth, wgpu's clip space: z 0..1). |
| `view_inverse` | `mat4x4<f32>` | 128 | View to camera-relative world space. |
| `projection_inverse` | `mat4x4<f32>` | 192 | Clip to view space. |
| `camera_position` | `vec3<f32>` | 256 | The camera's world position, in blocks (about a millimetre of precision 10 000 blocks out). |
| `time` | `f32` | 268 | Seconds since the pack was loaded, wrapping to 0 every hour. |
| `sun_direction` | `vec3<f32>` | 272 | Unit vector toward the sun (below the horizon at night). |
| `day_time` | `f32` | 284 | The time of day, 0..1: 0 sunrise, 0.25 noon, 0.5 sunset, 0.75 midnight. |
| `moon_direction` | `vec3<f32>` | 288 | Unit vector toward the moon. |
| `game_time` | `f32` | 300 | Vanilla's GameTime: (game time % 24000 + partial tick) / 24000, 0..1. |
| `sky_color` | `vec3<f32>` | 304 | Vanilla's sky colour this frame, 0..1. |
| `rain` | `f32` | 316 | Rain strength, 0..1. |
| `fog_color` | `vec3<f32>` | 320 | Vanilla's fog colour this frame (also the clear colour), 0..1. |
| `thunder` | `f32` | 332 | Thunder strength, 0..1. |
| `resolution` | `vec2<f32>` | 336 | The size of the world's picture, in pixels (the resolution scale applied). |
| `eye_light` | `vec2<f32>` | 344 | Block and sky light at the camera, 0..1 (level / 15). |
| `fog_start` | `f32` | 352 | Where vanilla's fog starts, in blocks from the camera. |
| `fog_end` | `f32` | 356 | Where vanilla's fog is full, in blocks. |
| `render_distance` | `f32` | 360 | The render distance, in blocks. |
| `far` | `f32` | 364 | The far plane, in blocks. |
| `night_vision` | `f32` | 368 | Night vision strength, 0..1. |
| `blindness` | `f32` | 372 | Blindness strength, 0..1. |
| `darkness` | `f32` | 376 | The darkness effect's strength, 0..1. |
| `brightness` | `f32` | 380 | The Brightness option, 0..1 (Moody to Bright). |
| `eye_in` | `u32` | 384 | What the camera is in: 0 air, 1 water, 2 lava, 3 powder snow. |
| `dimension` | `i32` | 388 | 0 the Overworld, -1 the Nether, 1 the End, 2 any other dimension. |
| `frame_index` | `u32` | 392 | Frames drawn since the pack was loaded (wraps). |
| `hide_gui` | `u32` | 396 | 1 while F1 hides the HUD, else 0. |

## Options

Each `[[options]]` entry sets the WGSL `override` of the same name when the pipelines are built:

| `type` | Keys | The `override` |
| --- | --- | --- |
| `bool` | `default` | `override <id>: bool` |
| `float` | `default`, `min`, `max`, `step` | `override <id>: f32` |
| `int` | `default`, `min`, `max` | `override <id>: i32` |
| `choice` | `values`, `default` (one of them) | `override <id>: u32`, the value's index |

## Limits

- 8 post passes, 64 options, `budget.gpu_ms` up to 16.
- 256 KiB per WGSL file, 8 MiB per pack.
- Per stage: 20000 naga expressions over all its functions, 64 texture reads (samples,
  gathers and loads, the prelude's included).

## The check

A pack runs only after it passes `horizon_shader_api::check` (the game when it loads a pack, the launcher before it
installs one, the registry's CI): each stage is parsed and validated by naga, checked against its signature and
the rules above, and translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL) with its options at their
defaults, minimums and maximums. Every option must be an `override` of its type in some stage; an `override`
that is no option needs a default.
