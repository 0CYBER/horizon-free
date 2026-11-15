# Shader API reference (0.2)

Generated from `crates/horizon-shader-api` (`HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test it reference::`
rewrites it; the test fails when it drifts). The API's design and pack format: [shaders.md](shaders.md).

**Experimental** ([stability.md](stability.md)): `horizon-shader check` takes 0.2 packs, the game draws them and
the registry lists them, and anything here may change in any release.
The design: [hdr-linear-light.md](../design/hdr-linear-light.md).

## Colour

Every colour a 0.2 stage reads or returns is **linear light**: BT.709 (sRGB) primaries by default, scene-referred,
1.0 is vanilla's white at paper white, more is brighter. A pack may ask for BT.2020 primaries, which hold colours
outside sRGB, with `[frame] primaries` (below). Horizon decodes vanilla's values with a 2.2 power (vanilla's, not
the piecewise sRGB curve), replacing negatives and NaN with 0 first, and encodes a stage's result the same way where
the frame stores gamma (`[frame] blend`, below).

- **Linear:** `MaterialInput` `albedo`, `tint`, `lightmap` and `colored_light`; `VertexInput.tint`; `SkyInput.color`;
  `WeatherInput.color`; `Frame` `sky_color` and `fog_color`; `block_texture`, `block_texture_level` and
  `block_light_color`; `RayHit` `albedo` and `tint`; every colour texture a stage samples (`scene_color`, `previous`,
  `history`, the pack's targets, `opaque_color`, `reflection`, `shadow_color`).
- **Data:** depth, normals, `light` levels, `emission`, every channel of the LabPBR maps, and the pack's
  `[[textures]]`.
- **`[[lights]]` and `[[particle_lights]]` colours** are written in vanilla's gamma, and Horizon decodes
  them: `block_light_color` and `colored_light` are linear. `block_light_colors` holds them as written; read it through
  `block_light_color`.
- **Alpha is coverage only**, never light: brightness is in rgb alone. A post pass's alpha is ignored.

## Room to grow

`Frame`, the stage inputs (`SkyInput`, `VertexInput`, `MaterialInput`, `DisplacementInput`, `WeatherInput`,
`PostInput`, `TonemapInput`, `EntityInput`), `surface_at`'s `SurfaceData` and `trace`'s `RayHit` may gain fields in later
releases. Read their fields and pass them along, but don't construct them: `horizon-shader check` refuses
`PostInput(...)` and the like. `Surface` is yours to construct and keeps its one field; later material outputs come as
helpers.

**New names reach a stage only if the pack asked for them.** A later release puts a new name before your files (a
helper, a struct, a resource, a constant) only under a manifest key the pack sets, or in a stage added later, whose
prelude is its own. A stage a pack has today keeps the names it has today, so a name of your own that is free now
stays yours: an addition doesn't break a pack that didn't ask for it. Names starting with `horizon_` are Horizon's
already, and new fields can't collide, since you don't construct those structs.

## The display

`frame.hdr`, `headroom`, `paper_white`, `peak` and `black_level` describe the display, read again every frame. With HDR
on, where the device keeps the HUD on a layer of its own, a 0.2 pack sees the display's: `hdr` 1, `paper_white` and
`peak` in nits, `headroom` the peak over paper white (at least 1) and `black_level` the display's black in nits.
Otherwise it sees SDR: `hdr` 0, `headroom` 1, `paper_white` and `peak` 80, `black_level` 0, and its picture ends on an
8-bit frame, clipped at white, before the camera overlays and the HUD draw. Horizon lifts the shadows onto
`black_level` after `[tonemap]`: read it, but don't apply it.

## `[frame] blend`

`blend = "gamma"` (the default) keeps the frame in vanilla's gamma, values above 1 kept: Horizon encodes the material,
sky and weather stages' results before they are stored and blended, so glass, water, clouds and particles blend as
vanilla's. `blend = "linear"` stores linear light, so see-through draws blend as light does: Horizon stores those
stages' results as they are and decodes its own draws (entities, particles, clouds, weather, the stages a pack leaves
out), so a solid surface looks as under `"gamma"`. A stage sees the same values either way. Where the device can't
draw a float frame the pack draws on an 8-bit one, in gamma. Under `"linear"` resource packs' `core/sky`,
`core/stars` and `core/rendertype_lines` shaders, which write gamma, give way to Horizon's own sky, stars and block
outline.

## `[frame] primaries`

`primaries = "bt709"` (the default) is the rule above. With `primaries = "bt2020"` every colour a stage reads or
returns is in BT.2020 primaries instead (D65 white, 1.0 still vanilla's white at paper white, values never negative).
It needs `blend = "linear"`, since a change of primaries is only right in linear light. `frame.primaries` says which
space the pack's colours are in, and `frame.luminance` holds that space's luminance weights: compute luminance as
`dot(color, frame.luminance)` and it is right in either. `"bt2020"` passes the check; the game doesn't serve it yet.

A stage is a function your WGSL defines, with the signature below; Horizon puts the prelude (these structs, the
resources and helpers) before your files and calls the function from its own pass. Your files declare no
bindings and no entry points, and no names starting with `horizon_`.

## `[material] coverage`

Which points of terrain are drawn is the engine's unless the pack says otherwise. Under `coverage = "engine"` (the
default) solid and cutout terrain keep Horizon's alpha test on the texture's own alpha (cut below 0.5, except the
leaves Horizon draws opaque), and the alpha `material` returns is ignored there; translucent terrain is cut where the
texture's alpha is below 0.01, and the returned alpha is its opacity, so water and glass still set how see-through
they are. `material`, and every function it calls, may not `discard`: the check refuses it. A pack that cuts its own
holes sets `coverage = "pack"`: the returned alpha then drives the alpha test (below 0.5 on solid and cutout
terrain, below 0.01 on translucent) and the stage may `discard`. Normal and parallax shading that keeps the
silhouette needs no `"pack"`, and neither do `[vertex]` and `[displacement]`, which move geometry, not coverage.
The stage writes no depth of its own either way.

## Stages

| Stage | In `horizon-shaders.toml` | Your function |
| --- | --- | --- |
| sky | `[sky]` | `fn sky(input: SkyInput) -> vec4<f32>` |
| vertex | `[vertex]` | `fn vertex(input: VertexInput) -> vec3<f32>` |
| material | `[material]` | `fn material(input: MaterialInput) -> Surface` |
| weather | `[weather]` | `fn weather(input: WeatherInput) -> vec4<f32>` |
| post | `[[post]]` | `fn post(input: PostInput) -> vec4<f32>` |
| tonemap | `[tonemap]` | `fn tonemap(input: TonemapInput) -> vec3<f32>` |
| entity | `[entity]` | `fn entity(input: EntityInput) -> Surface` |

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

## vertex

`fn vertex(input: VertexInput) -> vec3<f32>`

### `VertexInput`

One corner of a block's surface, for `fn vertex(input: VertexInput) -> vec3<f32>`, which returns where it is drawn (camera-relative, in blocks; `input.position` where it stays).

| Field | Type | Meaning |
| --- | --- | --- |
| `position` | `vec3<f32>` | The corner, camera-relative, in blocks (`+ frame.camera_position` for the world's). |
| `uv` | `vec2<f32>` | The texture coordinate inside the block's sprite. |
| `material` | `u32` | The `[[materials]]` id of the block's sprite (0 when none matches). |
| `light` | `vec2<f32>` | Block and sky light, 0..1. |
| `tint` | `vec4<f32>` | The vertex colour (biome tint, face shade, ambient occlusion); alpha 1. |
| `face` | `u32` | For a fluid, the face of the block the corner is on (as `MaterialInput.face`), else 0. |
| `fluid` | `u32` | 1 on a fluid's surface (water or lava), else 0. |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |

Helpers:

- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

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
| `tags` | `u32` | The pack's sprite tags this surface's sprite has: bit i for the manifest's i-th `[[tags]]`. |
| `material` | `u32` | The `[[materials]]` id of this surface's sprite (0 when no entry names it). |
| `screen` | `vec2<f32>` | Where the point is on the screen, 0..1 from the top left (`opaque_depth` and `opaque_color` at `screen * frame.resolution`). |
| `face` | `u32` | For a fluid, which face of the block the point is on, `Direction` order: 0 down, 1 up, 2 north, 3 south, 4 west, 5 east. 0 elsewhere. |
| `fluid` | `u32` | 0 not a fluid, 1 water, 2 lava. |
| `emission` | `f32` | How strongly the block itself shines, 0..1, by its kind of light: a redstone torch or a candle 7/15, a torch or lava 14/15, glowstone 1; 0 for most blocks. |
| `normal` | `vec3<f32>` | The surface's geometric normal, a unit vector in world axes, facing the camera. A normal within about 3.6 degrees of an axis is that axis exactly, so distant block faces do not shimmer; flowing water's slopes are steeper and keep theirs. |
| `tangent` | `vec4<f32>` | The surface's tangent: xyz the unit direction in world axes along which `uv.x` grows, w the sign (1 or -1) that makes `cross(normal, tangent.xyz) * tangent.w` the direction `uv.y` grows along (down the texture). With `normal` it turns a normal map's normals into world axes. |
| `colored_light` | `vec3<f32>` | The frame's coloured lights at this point (`[[particle_lights]]`, held and dropped items in their `[[lights]]` colour while dynamic lights are on, and mods' visual lights), 0..1, to light the surface with as the pack likes; 0 where none reaches. |
| `displaced` | `f32` | How much of the relief `[displacement]` gave this point as geometry, 0..1: 1 near the camera, where the surface already lies where `displacement` put it (`position` is that point; a parallax effect would count its depth twice), easing to 0 at `[displacement] distance`; always 0 without `[displacement]`. |

### `Surface`

What `material` returns. Vanilla's own is `albedo * tint * vec4(lightmap, 1.0)`; Horizon then applies the alpha test (cutout and translucent; on the texture's alpha unless `[material] coverage = "pack"`) and fog.

| Field | Type | Meaning |
| --- | --- | --- |
| `color` | `vec4<f32>` | The lit colour: rgb linear light, 0 or more and unbounded (more than 1 is brighter than white); alpha translucent terrain's opacity, and what the alpha test reads under `[material] coverage = "pack"`. |

Under `[displacement]` the stage also defines `fn displacement(input: DisplacementInput) -> f32`, called for each corner of the grid the faces near the camera are cut into; the corner moves along the face's normal by the depth it gives at the point's height less the depth at the texture's mean height, pinned at the face's edges and easing out over the last quarter of the distance.

### `DisplacementInput`

One corner of the grid a block face near the camera is cut into, for `fn displacement(input: DisplacementInput) -> f32` (`[displacement]`), which returns how deep below the face, in blocks, the surface lies at `height`.

| Field | Type | Meaning |
| --- | --- | --- |
| `position` | `vec3<f32>` | The corner on the flat face, camera-relative, in blocks. |
| `normal` | `vec3<f32>` | The face's normal, a unit vector in world axes. |
| `uv` | `vec2<f32>` | The texture coordinate inside the block's sprite (as `MaterialInput.uv`). |
| `material` | `u32` | The `[[materials]]` id of the block texture (0 when none matches). |
| `height` | `f32` | A LabPBR height, 0..1 (1 the top of the relief): the normal map's alpha here, or the texture's average height (what the face's edges are pinned to). |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |
| 1 | `var reflection: texture_2d<f32>` | The water mirror: the world as a camera mirrored about the horizontal water plane around the camera (`frame.water_plane`) sees it, at half resolution, mapped like the world's picture, so water samples it at `input.screen`. Solid terrain, cut away below the plane, over the vanilla sky gradient: not the pack's `sky` stage, nor the sun, moon and stars. Drawn only for a pack whose material reads it, while `frame.has_reflection` is 1; else a 1×1 blank. |
| 2 | `var reflection_sampler: sampler` | Bilinear filtering, clamped to the edges. |
| 3 | `var shadow_map: texture_depth_2d` | This frame's shadow map: the depth of the solid terrain and entities the light sees, 0 at the light, 1 away from it (and where nothing is), stored at `shadow_position`'s mapping. Valid while `frame.has_shadow` is 1 (the pack asks for it with `[shadow]`), else a 1×1 map that lights everything. |
| 4 | `var shadow_sampler: sampler_comparison` | Depth comparison, bilinear (2×2 percentage-closer filtering): `textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z)` is 1 where the point is lit, 0 in shadow. |
| 7 | `var shadow_map_translucent: texture_depth_2d` | `shadow_map` with the translucent terrain (water, stained glass, ice) the light passes through drawn in too, read with `shadow_sampler` the same way: a point lit in `shadow_map` but not here lies under such a surface, which `shadow_color` gives the colour of. |
| 8 | `var shadow_color: texture_2d<f32>` | The texture colour times the vertex colour (rgb: the tint, the face's directional shade and ambient occlusion) of the nearest translucent surface the light passes through at each texel of `shadow_map_translucent`, white where there is none; alpha is 1 on a fluid (water) and the texture's alpha, at most 0.99, on anything else. Read it with `textureLoad` at `shadow_position`'s xy times `textureDimensions(shadow_color)`. |
| 6 | `var opaque_depth: texture_depth_2d` | The world's depth before translucent terrain (reversed, like `scene_depth`), for the translucent pass: `textureLoad(opaque_depth, vec2<i32>(input.screen * frame.resolution), 0)` is what lies behind water or glass. Stale while solid terrain draws. |
| 9 | `var block_light_colors: texture_3d<f32>` | The colour of the block light around the camera, from the `[[lights]]` of the light sources nearby: rgb their colours averaged by how near each is, over 4; a how strongly they reach the cell. Read it through `block_light_color`. |
| 10 | `var<uniform> block_light_grid: vec4<f32>` | Where `block_light_colors` lies: xyz its corner, camera-relative, w the side of one of its cells in blocks. |
| 11 | `var opaque_color: texture_2d<f32>` | The world's picture before translucent terrain (sky, solid terrain, entities), the size of `opaque_depth`, for the translucent pass: what a surface refracts. Read it with `textureLoad`. Stale while solid terrain draws; black when the world draws straight to the window (a pack without `[[post]]` passes, at full resolution). |

Helpers:

- `fn block_texture(uv: vec2<f32>) -> vec4<f32>`: the surface's block texture at `uv`, anywhere on the sprite, without the tint.
- `fn block_texture_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at mip level `lod`.
- `fn block_texture_size() -> vec2<f32>`: its size in texels.
- `fn block_texture_lod() -> f32`: the mip level `block_texture` reads at this point.
- `fn block_pbr_maps() -> bool`: whether the resource packs ship LabPBR maps (`_n`, `_s`).
- `fn block_normal_map(uv: vec2<f32>) -> vec4<f32>`: the surface's LabPBR normal map at `uv` (rg the normal along `input.tangent` and down the texture, b ambient occlusion, a height); (0.5, 0.5, 1, 1) without one.
- `fn block_specular_map(uv: vec2<f32>) -> vec4<f32>`: its LabPBR specular map (r smoothness, g F0 or a metal, b porosity or subsurface, a emission, 1 none); (0, 0, 0, 0) without one.
- `fn block_normal_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`, `fn block_specular_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at the block texture's mip level `lod`.
- `fn block_pbr_size() -> vec2<f32>`: the LabPBR maps' size in texels.
- `fn set_surface(normal: vec3<f32>, smoothness: f32, data: f32)`: what the post stage's `surface_at` reads for this point of solid terrain (`data` the pack's own; the face's normal and 0, 0 unset).
- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

## weather

`fn weather(input: WeatherInput) -> vec4<f32>`

### `WeatherInput`

One point of a rain or snow streak, for `fn weather(input: WeatherInput) -> vec4<f32>`, which returns the colour blended over the world (alpha its opacity).

| Field | Type | Meaning |
| --- | --- | --- |
| `color` | `vec4<f32>` | The rain or snow texture's colour here times the streak's vertex colour (alpha 0.1..1). |
| `light` | `vec2<f32>` | Block and sky light, 0..1. |
| `position` | `vec3<f32>` | The point, camera-relative, in blocks. |
| `snow` | `bool` | A snow streak (rain otherwise). |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |

Helpers:

- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

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
| 6 | `var shadow_map: texture_depth_2d` | This frame's shadow map: the depth of the solid terrain and entities the light sees, 0 at the light, 1 away from it (and where nothing is), stored at `shadow_position`'s mapping. Valid while `frame.has_shadow` is 1 (the pack asks for it with `[shadow]`), else a 1×1 map that lights everything. |
| 7 | `var shadow_sampler: sampler_comparison` | Depth comparison, bilinear (2×2 percentage-closer filtering): `textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z)` is 1 where the point is lit, 0 in shadow. |
| 8 | `var history: texture_2d<f32>` | Last frame's input to the last pass (what the pass before it wrote), for temporal passes (TAA, denoising): put such a pass second to last and it reads its own previous output; reproject with `frame.previous_view_projection`. Black on the first frame, after a gap of over half a second, and with a single pass. |
| 9 | `var shadow_map_translucent: texture_depth_2d` | `shadow_map` with the translucent terrain (water, stained glass, ice) the light passes through drawn in too, read with `shadow_sampler` the same way: a point lit in `shadow_map` but not here lies under such a surface, which `shadow_color` gives the colour of. |
| 10 | `var shadow_color: texture_2d<f32>` | The texture colour times the vertex colour (rgb: the tint, the face's directional shade and ambient occlusion) of the nearest translucent surface the light passes through at each texel of `shadow_map_translucent`, white where there is none; alpha is 1 on a fluid (water) and the texture's alpha, at most 0.99, on anything else. Read it with `textureLoad` at `shadow_position`'s xy times `textureDimensions(shadow_color)`. |
| 11 | `var scene_surface: texture_2d<u32>` | What the material stage's `set_surface` stored for the solid terrain at each pixel, unfiltered (`textureLoad`; its texels are the bits of floats, so `bitcast<vec4<f32>>` them): read it through the prelude's `surface_at`, which knows its layout and where it no longer shows. Blank without a material stage that calls `set_surface`. |

Helpers:

- `fn depth_at(uv: vec2<f32>) -> f32`: the world's depth at `uv`.
- `fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32>`: the camera-relative position, in blocks, of
  the point at `uv` and `depth`.
- `fn surface_at(texel: vec2<i32>) -> SurfaceData`: the solid terrain's surface at a pixel, as the material stage's `set_surface` left it: `present` (false where the sky, an entity, water, glass or a particle shows instead), `normal` (world axes), `smoothness` and `data`.
- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

A `[[post]]` entry with `output = "<name>"` writes a target of the pack's own, at its `scale` of the
world's size (1, 0.5 or 0.25), instead of the next pass's `previous`; the prelude of every pass declares
`var <name>: texture_2d<f32>` (group 3 from binding 32), which holds last frame's until written again (a
pass reads a blank one in place of the target it writes).

### Ray tracing

With `[ray_tracing]` in the manifest (it has no keys, and needs a `[[post]]` pass or a `[material]` stage) the post
stage's prelude also has `RayHit` and the functions and constants below, so does the material stage's where its
files name one of the four trace functions, and the `Frame` of every stage of the pack has `has_ray_tracing` and
`ray_distance`. A pack without the key has none of them, and a material stage that names no trace function is
built as without it.

The game traces for the pack picked when it starts: only then, and only for a pack with `[ray_tracing]`, does it
ask the GPU for hardware ray queries, where the GPU and graphics API have them. In a game started without them a
pack picked while it runs traces from the next start, and the shader pack screen says so. `HORIZON_RAY_TRACING=1`
asks for ray queries and keeps the scene with any pack or none; any other value of it turns ray tracing off.

- `fn trace(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32) -> RayHit`: the nearest solid surface
  from `origin` (camera-relative, in blocks, as `view_position` gives it) along the unit vector `direction`, within
  `max_distance` blocks.
- `fn trace_any(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32) -> bool`: whether any solid surface
  is in the way: a shadow ray, which stops at the first it finds and reads nothing of it.
- `const RAY_SOLID: u32`, `const RAY_TRANSLUCENT: u32`: the scene's two layers, bits to OR together. Solid
  terrain, with cut-outs (leaves, plants) and lava; and what the translucent pass draws: water, glass, stained
  glass, ice, slime and honey blocks, any block whose texture has partly see-through texels.
- `fn trace_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> RayHit` and
  `fn trace_any_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> bool`: `trace`
  and `trace_any` over the layers `layers` names; `trace(o, d, m)` is `trace_layers(o, d, m, RAY_SOLID)`. With no
  layer named every ray misses.
- `fn hit_normal_map(hit: RayHit) -> vec4<f32>`, `fn hit_specular_map(hit: RayHit) -> vec4<f32>`: the resource
  packs' LabPBR maps at the hit, its sprite's texel at `hit.uv` at full size, as `block_normal_map_level(uv, 0.0)`
  and `block_specular_map_level(uv, 0.0)` read a drawn surface's; their defaults without maps, and on a miss.
- In the post stage, `fn block_pbr_maps() -> bool`: whether the resource packs ship LabPBR maps, as the material
  stage's.

A ray starts at `origin` itself, so move a point off its own surface before tracing from it. What stops it is what
the surface's own pass draws. In the solid layer it passes through the clear texels of a cut-out texture (leaves,
plants), as terrain's alpha test does: a texel stops it from alpha 0.5 up. In the translucent layer every texel
stops it, however faint, except a clear one (alpha below 0.01), which the translucent pass doesn't draw either;
the hit's `albedo.a` is the texel's alpha and its `translucent` is true. A ray over both layers stops at the
nearest surface of either: to go on through a translucent one, trace again from just behind the hit. The scene is
Horizon's: the terrain around the camera, out to `frame.ray_distance` at least, without entities, particles,
clouds or distant terrain. While
the game doesn't trace (`frame.has_ray_tracing` is 0) the pack is built and runs all the same, and every ray
misses. Your files name no ray query of their own (`ray_query`, `acceleration_structure`, `RayDesc`,
`RayIntersection`, a `rayQuery` function, `enable wgpu_ray_query`): the check refuses them, and an `output`
named as one of these functions.

In the material stage only `material` traces: `displacement` and the vertex stage's `vertex`, whose files share
its module, run in vertex shaders, and the check refuses a pack where either reaches a trace function.
`material` runs for every terrain fragment drawn, so a ray costs more there than in a post pass: keep its rays for
translucent surfaces (`input.translucent`), which leave no surface for a post pass to light.

### `RayHit`

The nearest surface along a ray, from `trace` or `trace_layers`. Its `albedo`, `tint`, `light`, `uv`, `material`, `tags`, `emission`, `fluid`, `translucent` and `tangent` are what `MaterialInput` gives the material stage for the same point, so one function can shade both. On a miss `distance` is negative and every other field is 0 (`translucent` false).

| Field | Type | Meaning |
| --- | --- | --- |
| `distance` | `f32` | How far along the ray the surface is, in blocks; negative on a miss. |
| `position` | `vec3<f32>` | The point, camera-relative, in blocks (as `view_position`). |
| `normal` | `vec3<f32>` | The face's normal, a unit vector in world axes, turned toward the ray's origin. |
| `uv` | `vec2<f32>` | The texture coordinate inside the block's sprite (as `MaterialInput.uv`). |
| `albedo` | `vec4<f32>` | The block texture's colour at the point, at full size (mip level 0), linear light; alpha the texture's. |
| `tint` | `vec4<f32>` | The vertex colour: biome tint, face shade and ambient occlusion, linear light; alpha 1. |
| `light` | `vec2<f32>` | Block and sky light, 0..1, smoothed over the face. |
| `material` | `u32` | The `[[materials]]` id of the surface's sprite (0 when no entry names it). |
| `tags` | `u32` | The pack's sprite tags the surface's sprite has: bit i for the manifest's i-th `[[tags]]`. |
| `emission` | `f32` | How strongly the block itself shines, 0..1, by its kind of light: a redstone torch or a candle 7/15, a torch or lava 14/15, glowstone 1; 0 for most blocks (as `MaterialInput.emission`). |
| `fluid` | `u32` | 0 not a fluid, 1 water, 2 lava (as `MaterialInput.fluid`). Water is in the `RAY_TRANSLUCENT` layer, lava in `RAY_SOLID`. |
| `translucent` | `bool` | A surface of the translucent pass (water, stained glass, ice), as `MaterialInput.translucent`: a hit in the `RAY_TRANSLUCENT` layer, so false for every hit of `trace`. |
| `tangent` | `vec4<f32>` | The surface's tangent, as `MaterialInput.tangent`: xyz the unit direction in world axes along which `uv.x` grows, w the sign (1 or -1) that makes `cross(normal, tangent.xyz) * tangent.w` the direction `uv.y` grows along (down the texture). It follows `normal`, which faces the ray's origin. With `normal` it turns `hit_normal_map`'s normals into world axes. |
| `horizon_sprite` | `u32` | Horizon's own: where the hit's sprite is kept, for `hit_normal_map` and `hit_specular_map`. Not for packs to read; it changes with the renderer. |

Bindings 48 to 59 of group 3 hold the scene while the game traces, and in a post pass bindings
12 and 13 the LabPBR maps (the material stage has them there already). They are Horizon's, as their
`horizon_` names say, and change with the renderer.

## Kept resources and compute passes

**Checked, not run yet:** `horizon-shader check` takes these keys, and the game refuses a pack that declares a
`[[resource]]`, a `[[before_world]]` entry or a compute `[[post]]` entry: it needs a later Horizon.

- `[[resource]]`: `name`; `buffer` (bytes) with `element` (`u32`, `atomic<u32>`, `f32`, `vec4<u32>`, `vec4<f32>`), or `format`
  (`rgba16float`, `r32float`, `rgba32float`, `rgba32uint`) with `size = [w, h]` texels or `scale` (1, 0.5 or 0.25 of the render size); `keep`, `"view"`
  (screen history) or `"world"` (a cache, fixed `size` only, which needs `[kept] reach`: a teleport further, in
  blocks, clears it). A texture has this frame's version, `<name>`, and last frame's, `<name>_previous`; a buffer
  has one. Both names are free WGSL names, as an `output`'s, and no word WGSL reserves.
- `[[before_world]]`: a compute pass once a frame, before the world draws: `file`, `dispatch` (`[x, y, z]`
  cells, or `"pixels"`, one per texel of the render size), `workgroup = [x, y, z]`, `reads` and `writes`.
- `[[post]]` with `compute = true` and the same keys: a dispatch in the chain where it stands. It has no `output`
  or `scale`, isn't the last entry and doesn't advance `previous`. An ordinary pass lists what it `reads`, and its
  `output` may name a kept texture, which it draws at that texture's size.
- An option's `resets = "view"` clears the view's kept resources when the player changes it, `"world"` all
  of them.

A pass lists every resource it uses: in `reads` this frame's `x` or last frame's `x_previous`, in `writes` (a
compute pass's) `x`, once. An `output` is its pass's write. Its prelude declares only what it lists, so a name
it doesn't list is the file's own, and using such a resource is refused as unlisted. The check also refuses a
compute pass that writes a texture and reads it back (read `_previous`, or a buffer), and a pass that reads its
own `output`.

A compute file defines `fn compute(input: ComputeInput)`, and Horizon adds the `@compute` entry with the pass's
workgroup size. Every invocation of every workgroup runs it, so `var<workgroup>` and `workgroupBarrier()`
reductions stay uniform: one beyond the dispatch has `in_bounds` false and touches no resource. A compute
`[[post]]` pass's prelude is the post stage's with `ComputeInput` for `PostInput`; a `[[before_world]]` pass has
the frame, the samplers and the shadow maps (bindings below) with `shadow_position`.

**Not yet, in later steps:** a compute or `[[before_world]]` file doesn't get `trace` or the other ray functions
under `[ray_tracing]`, and `[material]` can't read kept resources.

### `ComputeInput`

One invocation of a dispatch, for `fn compute(input: ComputeInput)`.

| Field | Type | Meaning |
| --- | --- | --- |
| `cell` | `vec3<u32>` | The invocation's cell: where it is in the whole dispatch. |
| `local` | `u32` | Its index in its workgroup, from 0 to the workgroup's invocations less 1. |
| `size` | `vec3<u32>` | The dispatch's cells: its `dispatch`, or the render size and 1 for `"pixels"`. |
| `in_bounds` | `bool` | Whether `cell` is within `size`. Every invocation of every workgroup runs: one beyond `size` touches no resource, but still reaches every `workgroupBarrier()`. |

A pass's prelude declares the resources it lists, the pack's resource i at binding 64 + 2i of group
3 and its `_previous` at 65 + 2i: a buffer `var<storage, read_write> x: array<element>` where written,
`var<storage, read>` where only read (an `atomic<u32>` buffer then reads as `array<u32>`); a texture
`texture_storage_2d<format, write>` where written, else `texture_2d<f32>` (`texture_2d<u32>` for
`rgba32uint`, which only a compute pass writes). An `r32float`, `rgba32float` or `rgba32uint` texture is read
with `textureLoad` only: the check refuses sampling or gathering one, also through a function's argument. Data is data: Horizon doesn't decode, clamp or force the alpha of a kept resource.

`[[before_world]]` resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |
| 4 | `var linear_sampler: sampler` | Bilinear filtering, clamped to the edges. |
| 5 | `var nearest_sampler: sampler` | Nearest texel, clamped to the edges. |
| 6 | `var shadow_map: texture_depth_2d` | This frame's shadow map: the depth of the solid terrain and entities the light sees, 0 at the light, 1 away from it (and where nothing is), stored at `shadow_position`'s mapping. Valid while `frame.has_shadow` is 1 (the pack asks for it with `[shadow]`), else a 1×1 map that lights everything. |
| 7 | `var shadow_sampler: sampler_comparison` | Depth comparison, bilinear (2×2 percentage-closer filtering): `textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z)` is 1 where the point is lit, 0 in shadow. |
| 9 | `var shadow_map_translucent: texture_depth_2d` | `shadow_map` with the translucent terrain (water, stained glass, ice) the light passes through drawn in too, read with `shadow_sampler` the same way: a point lit in `shadow_map` but not here lies under such a surface, which `shadow_color` gives the colour of. |
| 10 | `var shadow_color: texture_2d<f32>` | The texture colour times the vertex colour (rgb: the tint, the face's directional shade and ambient occlusion) of the nearest translucent surface the light passes through at each texel of `shadow_map_translucent`, white where there is none; alpha is 1 on a fluid (water) and the texture's alpha, at most 0.99, on anything else. Read it with `textureLoad` at `shadow_position`'s xy times `textureDimensions(shadow_color)`. |

A kept resource starts as zeros and is zeros again when cleared: a `keep = "view"` one when
`frame.view_epoch` changes, every one when `frame.world_epoch` does (never 0, so a cleared value never
matches). Both change before the frame that sees them draws, and `frame.reset_reasons` has a bit for each
reason, a `const` of the prelude (`frame.reset_reasons & REASON_TELEPORT`); only a pack with `[[resource]]`
has these fields and constants. A world cache keyed by `world_epoch` survives a camera cut. Nothing else
says the world changed, not the camera's distance or `frame_index`.

| Reason | Bit | Clears | When |
| --- | --- | --- | --- |
| `REASON_LOAD` | 1 | both | The pack was loaded or reloaded: its first frame. |
| `REASON_RESET` | 2 | both | Horizon restarted the pack's history (a screenshot sequence's first shot). |
| `REASON_WORLD` | 4 | both | Another world or dimension, or its meshes were rebuilt (a render setting). |
| `REASON_TELEPORT` | 8 | both | The camera moved further than `[kept] reach` since last frame. |
| `REASON_PAUSE` | 16 | view | Over half a second since last frame, as `delta_time` 0. |
| `REASON_SIZE` | 32 | view | The render size or the window's changed. |
| `REASON_DISPLAY` | 64 | view | The output turned HDR or SDR. |
| `REASON_OPTION` | 128 | its `resets` | The player changed an option marked `resets`. |

Limits: 8 resources, 4 `[[before_world]]` entries, 8 compute
`[[post]]` entries; 32 MiB of buffers and 1024 KiB of fixed-size textures, both versions counted; each R-relative
kept texture takes 2 of the post targets' budget (Limits); a workgroup of up to [256, 256, 64] and
256 invocations; 33554432 invocations a dispatch and 67108864 a frame over every
compute pass, `[[before_world]]` and `[[post]]` together, counted as whole workgroups, a `"pixels"` dispatch as 3840×2160 (R isn't known at load).

### Motion guide

How each pixel's surface moved since last frame, for temporal passes (TAA, denoisers, light caches) that reproject
`history` or a target of their own. With `[motion]` in the manifest (it has no keys, and needs a `[[post]]` pass)
every post pass's prelude also has the names below; a pack without the key has none of them, and no other stage does.
`horizon-shader check` takes it; **the game doesn't draw it yet** and refuses a pack with `[motion]`.

- `var scene_motion: texture_2d<f32>` (binding 14): the engine's objects (z 1 matched, xy the delta; w 1
  nothing to match), and at depth 0 the sky's motion (xy; w 1 behind last frame's view). Read it through `motion_at`.
- `fn motion_at(texel: vec2<i32>) -> MotionData`: the motion at a pixel of the world's picture, as `scene_depth` holds it.
- `struct MotionData { delta: vec2<f32>, kind: u32 }`. `delta` is where the surface was last frame less where it is
  now, in top-left UV of the world's picture (`frame.resolution`): `uv + delta` is its place in last frame's picture.
  `kind` says how far to trust it:
  - `MOTION_CAMERA` (0): everything that isn't one of the engine's objects (terrain, still block entities, the sky),
    moved by the camera alone, through `scene_depth`, `frame.previous_view_projection` and `frame.camera_motion`. The
    sky (depth 0) is drawn without the view bob, so Horizon writes its motion into `scene_motion` itself, from the
    unbobbed views. It assumes still geometry: terrain a `[vertex]` stage animates, drifting clouds and
    moving water are wrong here. Mark such surfaces with `set_surface`'s `data` and treat them as reactive.
  - `MOTION_OBJECT` (1): one of the engine's objects matched to last frame's, vertex by vertex, limbs and spin
    included: mobs, dropped items, boats, carts, item frames and their items, armour stands, piston blocks, animated
    block entities, the player in third person.
  - `MOTION_NEW` (2): one of the engine's objects with no usable last frame (new, changed shape, or unmarked, such as
    an opaque particle). `delta` is 0: reject history there.
  - `MOTION_NO_HISTORY` (3): every pixel on the first frame and after a gap (`frame.delta_time` 0), and a point last
    frame's camera had behind it. `delta` is 0.
- The vectors come from the unjittered projections, so they line up with `scene_depth`.
- Not covered: the first-person hand (drawn after the pack's tone map), translucent surfaces that write no depth
  (translucent items, most particles, weather), and terrain moved by a `[vertex]` stage, whose last place Horizon
  doesn't know.

## tonemap

`fn tonemap(input: TonemapInput) -> vec3<f32>`

### `TonemapInput`

One pixel of an output (the window), for `fn tonemap(input: TonemapInput) -> vec3<f32>`, which returns its colour for the output `frame` describes: linear light, 1.0 at paper white, from 0 to `frame.headroom` (Horizon clamps it there).

| Field | Type | Meaning |
| --- | --- | --- |
| `uv` | `vec2<f32>` | Where the pixel is, 0..1 from the top left. |
| `pixel` | `vec2<f32>` | The pixel's centre, in pixels from the top left of the output (the window). |
| `color` | `vec3<f32>` | The post chain's picture here (the world's without one), linear light, never negative or NaN. |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |
| 3 | `var previous: texture_2d<f32>` | The post chain's picture at the window's size (without a post chain, the world's at the world's size), linear light: what `input.color` samples, for a curve that reads around the pixel. |
| 4 | `var linear_sampler: sampler` | Bilinear filtering, clamped to the edges. |
| 5 | `var nearest_sampler: sampler` | Nearest texel, clamped to the edges. |

It is a pure mapping from the scene's light to the output its `frame` describes, run once per pixel of
each output, after the post chain and before the camera overlays and the HUD. Today the display is the
only output; Horizon may add others for the same frame, such as an SDR screenshot or icon, where `frame`'s
display fields describe SDR (`hdr` 0, `headroom` 1, `paper_white` and `peak` 80, `black_level` 0). Its
only output is the colour it returns, so nothing carries from one call to the next. A pack without it gets
Horizon's: a clip at white in SDR, and in HDR the curve Horizon uses without a pack, which keeps light up
to 60% of the peak as it is and rolls it off into the peak above. Either runs once: Horizon doesn't map a pack's
result again. Horizon holds the result to 0 to `frame.headroom`, then lifts the shadows onto
`frame.black_level`.

## entity

`fn entity(input: EntityInput) -> Surface`

### `EntityInput`

One point of the surface of an entity, a block entity, an item or a particle, for `fn entity(input: EntityInput) -> Surface`.

| Field | Type | Meaning |
| --- | --- | --- |
| `albedo` | `vec4<f32>` | The texture's colour here. |
| `tint` | `vec4<f32>` | The vertex colour without vanilla's directional shade (dyes, particle colours, fades). |
| `overlay` | `vec4<f32>` | The flash vanilla lays over the surface (hurt, TNT, a creeper's swelling): rgb its colour, alpha how much of the surface it replaces. Alpha 0 without one. |
| `lightmap` | `vec3<f32>` | Vanilla's light colour for `light` this frame. |
| `shade` | `f32` | Vanilla's directional shade for this face as a factor on linear light, 0..1 (vanilla's value decoded with the 2.2 power, as every 0.2 colour is). 1 where vanilla applies none. |
| `position` | `vec3<f32>` | The point, camera-relative, in blocks. |
| `emission` | `f32` | 1 on a layer vanilla draws at full brightness (spider and enderman eyes, the warden's glow), else 0. There `lightmap` is full light; `shade` stays vanilla's, which shades the warden's glow and not the eyes. |
| `normal` | `vec3<f32>` | The surface's geometric normal, a unit vector in world axes, facing the camera, from the position's screen derivatives (as `MaterialInput.normal`, without its snap to the axes). |
| `kind` | `u32` | What is drawn: 0 an entity (mobs, players, armour, vehicles), 1 a block entity (chests, banners, bells, shulker boxes), 2 a dropped or framed item, 3 reserved for the first-person hand, 4 a particle. |
| `colored_light` | `vec3<f32>` | As `MaterialInput.colored_light`: the frame's coloured lights at this point, 0..1; 0 where none reaches. |
| `translucent` | `bool` | Drawn with blending. |
| `light` | `vec2<f32>` | Block and sky light, 0..1. |
| `uv` | `vec2<f32>` | The texture coordinate. |
| `screen` | `vec2<f32>` | Where the point is on the screen, 0..1 from the top left (as `MaterialInput.screen`). |

`entity` lights what is no block terrain: `input.kind` says which. It returns the same `Surface` as `material` (rgb
linear light, 0 or more and unbounded; alpha its coverage), so a pack writes its lighting once and calls it from both
stages. Around it Horizon keeps vanilla's order: the alpha test on the returned alpha, then the enchantment glint
added on top, then fog. A pack without `[entity]` is drawn as before, and sees none of this stage's names; the rule
for new names holds for the stage as for the others.

**What the game draws through the stage so far: entities and block entities** (`kind` 0 and 1). Until a later
release brings them in, Horizon lights the rest itself, exactly as without the stage: dropped and framed items and
what a block or a display entity shows of one (`kind` 2), what flies as its item (a snowball, a potion, a fireball,
an eye of ender), firework rockets, particles (`kind` 4), the fire on a burning entity, a guardian's beam, a lead's
rope, name tags and text, and the first-person hand. Some draws stay Horizon's for good: beacon beams, the beams of
End crystals, End portals and gateways, breaking cracks, energy swirls, lightning and other additive glows.

An item an entity holds or wears as its model (a sword in a hand, a block on a head) reaches the stage with the
entity, unless its texture has see-through pixels (stained glass, ice, a slime block): that one is drawn with the
translucent items, and stays Horizon's until items draw through the stage. A block drawn as an entity (a falling
block, primed TNT, a block display) or as a block entity (a block a piston moves) reaches the stage too. These have
vanilla's shade already in `tint`, and `shade` 1: their vertices carry the two as one colour so far. A sign's board
and a bed are block terrain, the material stage's.

Vanilla's own, the picture a pack without the stage gets:

```wgsl
fn entity(input: EntityInput) -> Surface {
    let base = input.albedo.rgb * input.tint.rgb * input.shade;
    // Vanilla lays the flash over the surface in its gamma (the 2.2 power), before the light.
    let gamma = vec3<f32>(1.0 / 2.2);
    let flashed = pow(mix(pow(base, gamma), pow(input.overlay.rgb, gamma), input.overlay.a), vec3<f32>(2.2));
    return Surface(vec4<f32>(flashed * input.lightmap, input.albedo.a * input.tint.a));
}
```

Without a flash (`overlay.a` 0) that is `albedo * tint * shade * lightmap`. In linear light alone,
`mix(base, input.overlay.rgb, input.overlay.a) * input.lightmap` is the same flash a little brighter. The shade and
the light act on rgb only. `lightmap` holds no coloured light: where `colored_light` is above 0, Horizon's own draw
adds it to the light colour, and the expression above leaves it out.

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |
| 2 | `var block_light_sampler: sampler` | Bilinear filtering, clamped to the edges: what `block_light_color` reads `block_light_colors` with. |
| 3 | `var shadow_map: texture_depth_2d` | This frame's shadow map: the depth of the solid terrain and entities the light sees, 0 at the light, 1 away from it (and where nothing is), stored at `shadow_position`'s mapping. Valid while `frame.has_shadow` is 1 (the pack asks for it with `[shadow]`), else a 1×1 map that lights everything. |
| 4 | `var shadow_sampler: sampler_comparison` | Depth comparison, bilinear (2×2 percentage-closer filtering): `textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z)` is 1 where the point is lit, 0 in shadow. |
| 7 | `var shadow_map_translucent: texture_depth_2d` | `shadow_map` with the translucent terrain (water, stained glass, ice) the light passes through drawn in too, read with `shadow_sampler` the same way: a point lit in `shadow_map` but not here lies under such a surface, which `shadow_color` gives the colour of. |
| 8 | `var shadow_color: texture_2d<f32>` | The texture colour times the vertex colour (rgb: the tint, the face's directional shade and ambient occlusion) of the nearest translucent surface the light passes through at each texel of `shadow_map_translucent`, white where there is none; alpha is 1 on a fluid (water) and the texture's alpha, at most 0.99, on anything else. Read it with `textureLoad` at `shadow_position`'s xy times `textureDimensions(shadow_color)`. |
| 9 | `var block_light_colors: texture_3d<f32>` | The colour of the block light around the camera, from the `[[lights]]` of the light sources nearby: rgb their colours averaged by how near each is, over 4; a how strongly they reach the cell. Read it through `block_light_color`. |
| 10 | `var<uniform> block_light_grid: vec4<f32>` | Where `block_light_colors` lies: xyz its corner, camera-relative, w the side of one of its cells in blocks. |

Helpers:

- `fn block_light_color(position: vec3<f32>) -> vec4<f32>`: the colour of the block light at the camera-relative `position` (rgb) and how strongly it reaches it (a), as the material stage's.
- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

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
| `precipitation` | `u32` | 400 | What falls in the biome at the camera while it rains: 0 nothing, 1 rain, 2 snow. |
| `temperature` | `f32` | 404 | The biome at the camera's temperature (vanilla's, -0.7..2.0; it snows below 0.15). |
| `downfall` | `f32` | 408 | The biome at the camera's downfall, 0..1. |
| `wetness` | `f32` | 412 | `rain`, smoothed: rises over about 30 seconds and dries over about 10, 0..1. |
| `previous_view_projection` | `mat4x4<f32>` | 416 | Last frame's camera-relative world to clip space (its `projection * view`); a point at camera-relative `p` now was at `p + camera_motion` relative to last frame's camera. This frame's on the first frame and after a gap of over half a second (a pause, a world load), when last frame's camera says nothing about this one. |
| `camera_motion` | `vec3<f32>` | 480 | How far the camera moved since last frame, in blocks (0 after such a gap). |
| `delta_time` | `f32` | 492 | Seconds since last frame (0 on the first and after such a gap). |
| `eye_light_smooth` | `vec2<f32>` | 496 | `eye_light` eased with a half-life of 2 seconds: it follows the light levels' steps without stepping itself (for exposure and indoor fog). |
| `in_water` | `f32` | 504 | 1 while the camera is in water (`eye_in` 1), eased with a half-life of a quarter second in and out. |
| `in_lava` | `f32` | 508 | The same for lava (`eye_in` 2). |
| `water_plane` | `f32` | 512 | The y of the horizontal water plane `reflection` mirrors the world about, in world blocks, while `has_reflection` is 1. |
| `has_reflection` | `u32` | 516 | 1 while the material stage's `reflection` holds this frame's water mirror, else 0. |
| `reflection_resolution` | `vec2<f32>` | 520 | The size of `reflection`, in pixels: half the world's picture. |
| `mods` | `array<vec4<f32>, 8>` | 528 | Eight values a native mod hands its own pack each frame (`native_shader_params`, docs/native-mods.md): what they mean is that mod's, such as where and when something happened in the world. All 0 when no mod sets them. |
| `light_direction` | `vec3<f32>` | 656 | Unit vector toward the body casting this frame's shadows: the sun by day, the moon by night. |
| `has_shadow` | `u32` | 668 | 1 while `shadow_map` holds this frame's shadow map, else 0. |
| `shadow_view_proj` | `mat4x4<f32>` | 672 | Camera-relative world to the shadow camera's clip space, before the distortion (orthographic, looking along `-light_direction`; z 0 at the light, 1 away from it). |
| `shadow_distance` | `f32` | 736 | Half the side of the square the shadow map covers, in blocks (the manifest's `[shadow] distance`); 0 without a shadow map. |
| `shadow_distortion` | `f32` | 740 | How strongly the shadow map is distorted toward its centre, 0..1: a point at shadow clip `xy` is stored at `xy / (length(xy) * k + 1 - k)`, so texels near the camera cover less ground (`shadow_position` applies it). |
| `shadow_resolution` | `f32` | 744 | The shadow map's side, in texels; 0 without a shadow map. |
| `shadow_center_texel` | `f32` | 748 | The ground one shadow texel covers at the map's centre, in blocks (for a bias that scales with it); 0 without a shadow map. |
| `hdr` | `u32` | 752 | 1 while the output is HDR, else 0. |
| `headroom` | `f32` | 756 | The peak over paper white: the brightest value the display shows (Apple's EDR headroom, Android's HDR/SDR ratio). 1 in SDR. It changes while the game runs (the system's SDR white, a calibration, another display), so read it every frame. |
| `paper_white` | `f32` | 760 | Paper white in nits; a nominal 80 in SDR. |
| `peak` | `f32` | 764 | The peak brightness in nits; a nominal 80 in SDR, as `paper_white`. |
| `black_level` | `f32` | 768 | The display's black in nits (HDR's Black Level); 0 in SDR. |
| `primaries` | `u32` | 772 | The primaries of this pack's colours (`[frame] primaries`): 0 BT.709, 1 BT.2020. |
| `has_ray_tracing` | `u32` | 776 | 1 while `trace` and `trace_any` read this frame's scene, else 0 (a GPU without ray queries, ray tracing off, no world): every ray then misses. Only a pack with `[ray_tracing]` has this field. |
| `ray_distance` | `f32` | 780 | How far from the camera along x and along z, in blocks, the scene is sure to reach; 0 while `has_ray_tracing` is 0. Only a pack with `[ray_tracing]` has this field. |
| `luminance` | `vec3<f32>` | 784 | The weights that give a colour's luminance in those primaries, `dot(c, frame.luminance)`: (0.2126, 0.7152, 0.0722) for BT.709, (0.2627, 0.6780, 0.0593) for BT.2020. |
| `view_epoch` | `u32` | 796 | Changes when the pack's `keep = "view"` resources are cleared, to zeros (`reset_reasons` says why), and is never 0. Only a pack with `[[resource]]` has this field. |
| `world_epoch` | `u32` | 800 | The same for its `keep = "world"` resources. `view_epoch` changes with it: what clears the world's clears the view's too. Only a pack with `[[resource]]` has this field. |
| `reset_reasons` | `u32` | 804 | Why an epoch changed this frame: the prelude's `REASON_*` bits ("Kept resources"); 0 when neither did. Only a pack with `[[resource]]` has this field. |

## Anti-aliasing

While a pack's `[[post]]` chain draws the frame, the player's anti-aliasing settings do not run: `taa`, `fxaa`
and `upscaling` in `horizon-graphics.txt` have no effect, and the frame's projection is never jittered, because the
last post pass draws the picture itself and Horizon's own resolve, which does TAA, FXAA and TAA upscaling, is the
step it replaces. The world is still rendered at `resolutionScale` before the pack reads it.

A pack that wants smooth edges draws them itself, in a post pass: a spatial filter (FXAA, SMAA and the like) works on
`previous` as it is. A temporal pass cannot do what TAA does here, even though `history` and
`frame.previous_view_projection` give it the last frame: nothing moves the camera by a fraction of a texel from one
frame to the next, so a still camera gives it the same picture twice and there is no sub-pixel detail to gather.

Without a pack the settings apply as the player set them.

## Options

Each `[[options]]` entry sets the WGSL `override` of the same name when the pipelines are built:

| `type` | Keys | The `override` |
| --- | --- | --- |
| `bool` | `default` | `override <id>: bool` |
| `float` | `default`, `min`, `max`, `step` | `override <id>: f32` |
| `int` | `default`, `min`, `max` | `override <id>: i32` |
| `choice` | `values`, `default` (one of them) | `override <id>: u32`, the value's index |

## Limits

- 12 post passes, 64 options, `budget.gpu_ms` up to 16.
- Post targets: 16 `output`s, and 6 render-size `rgba16float` textures of memory with the
  R-relative kept textures: an `output` takes its `scale`² (1, 0.25 or 0.0625), an R-relative kept texture 2 (its
  two versions, at any format and scale).
- 256 KiB per WGSL file, 8 MiB per pack.
- Per stage: 20000 naga expressions over all its functions, 64 texture reads (samples,
  gathers and loads, the prelude's included).

## The check

A pack runs only after it passes `horizon_shader_api::check` (the game when it loads a pack, the launcher before it
installs one, the registry's CI): each stage is parsed and validated by naga, checked against its signature and
the rules above, and translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL) with its options at their
defaults, minimums and maximums. Every option must be an `override` of its type in some stage; an `override`
that is no option needs a default.
