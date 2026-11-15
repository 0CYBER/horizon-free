//! docs/api/shaders-reference-0.2.md, generated from the stages and inputs this crate defines
//! (`tests/reference.rs` fails when the file drifts).
use crate::check::{MAX_EXPRESSIONS, MAX_TEXTURE_READS};
use crate::inputs::{COMPUTE_INPUT, DISPLACEMENT_INPUT, Frame, RAY_HIT, SURFACE, Struct};
use crate::kept::{
    Element, Format, MAX_BEFORE_WORLD, MAX_BUFFER_BYTES, MAX_COMPUTE_POST,
    MAX_DISPATCH_INVOCATIONS, MAX_FIXED_TEXTURE_BYTES, MAX_FRAME_INVOCATIONS, MAX_RESOURCES,
    MAX_WORKGROUP, MAX_WORKGROUP_INVOCATIONS, PIXELS_COUNTED_AS, REASONS,
};
use crate::manifest::{API, MAX_GPU_MS, MAX_OPTIONS, MAX_POST_PASSES};
use crate::package::{MAX_TOTAL, MAX_WGSL};
use crate::stages::{
    GROUP, KEPT_BINDING, MOTION_BINDING, PBR_NORMALS_BINDING, PBR_SPECULAR_BINDING,
    RAY_SCENE_BINDING, RAY_SPRITE_ARRAYS, RAY_SPRITES_BINDING, StageKind,
};

fn table(s: &Struct, offsets: Option<&[usize]>) -> String {
    let mut out = format!("### `{}`\n\n{}\n\n", s.name, one_line(s.doc));
    out += if offsets.is_some() {
        "| Field | Type | Offset | Meaning |\n| --- | --- | --- | --- |\n"
    } else {
        "| Field | Type | Meaning |\n| --- | --- | --- |\n"
    };
    for (i, f) in s.fields.iter().enumerate() {
        let offset = offsets.map_or(String::new(), |o| format!(" {} |", o[i]));
        out += &format!(
            "| `{}` | `{}` |{offset} {} |\n",
            f.name,
            f.ty,
            one_line(f.doc)
        );
    }
    out + "\n"
}

fn one_line(doc: &str) -> String {
    doc.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the reference lives, from the repository's root.
pub const PATH: &str = "docs/api/shaders-reference-0.2.md";

/// The line's rules, after the introduction.
const RULES: &str = "\
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

`blend = \"gamma\"` (the default) keeps the frame in vanilla's gamma, values above 1 kept: Horizon encodes the material,
sky and weather stages' results before they are stored and blended, so glass, water, clouds and particles blend as
vanilla's. `blend = \"linear\"` stores linear light, so see-through draws blend as light does: Horizon stores those
stages' results as they are and decodes its own draws (entities, particles, clouds, weather, the stages a pack leaves
out), so a solid surface looks as under `\"gamma\"`. A stage sees the same values either way. Where the device can't
draw a float frame the pack draws on an 8-bit one, in gamma. Under `\"linear\"` resource packs' `core/sky`,
`core/stars` and `core/rendertype_lines` shaders, which write gamma, give way to Horizon's own sky, stars and block
outline.

## `[frame] primaries`

`primaries = \"bt709\"` (the default) is the rule above. With `primaries = \"bt2020\"` every colour a stage reads or
returns is in BT.2020 primaries instead (D65 white, 1.0 still vanilla's white at paper white, values never negative).
It needs `blend = \"linear\"`, since a change of primaries is only right in linear light. `frame.primaries` says which
space the pack's colours are in, and `frame.luminance` holds that space's luminance weights: compute luminance as
`dot(color, frame.luminance)` and it is right in either. `\"bt2020\"` passes the check; the game doesn't serve it yet.

";

/// `[ray_tracing]`, at the end of the post stage.
fn ray_tracing() -> String {
    let mut out = String::from(
        "### Ray tracing\n\n\
         With `[ray_tracing]` in the manifest (it has no keys, and needs a `[[post]]` pass or a `[material]` stage) the post\n\
         stage's prelude also has `RayHit` and the functions and constants below, so does the material stage's where its\n\
         files name one of the four trace functions, and the `Frame` of every stage of the pack has `has_ray_tracing` and\n\
         `ray_distance`. A pack without the key has none of them, and a material stage that names no trace function is\n\
         built as without it.\n\n\
         The game traces for the pack picked when it starts: only then, and only for a pack with `[ray_tracing]`, does it\n\
         ask the GPU for hardware ray queries, where the GPU and graphics API have them. In a game started without them a\n\
         pack picked while it runs traces from the next start, and the shader pack screen says so. `HORIZON_RAY_TRACING=1`\n\
         asks for ray queries and keeps the scene with any pack or none; any other value of it turns ray tracing off.\n\n\
         - `fn trace(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32) -> RayHit`: the nearest solid surface\n  \
         from `origin` (camera-relative, in blocks, as `view_position` gives it) along the unit vector `direction`, within\n  \
         `max_distance` blocks.\n\
         - `fn trace_any(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32) -> bool`: whether any solid surface\n  \
         is in the way: a shadow ray, which stops at the first it finds and reads nothing of it.\n\
         - `const RAY_SOLID: u32`, `const RAY_TRANSLUCENT: u32`: the scene's two layers, bits to OR together. Solid\n  \
         terrain, with cut-outs (leaves, plants) and lava; and what the translucent pass draws: water, glass, stained\n  \
         glass, ice, slime and honey blocks, any block whose texture has partly see-through texels.\n\
         - `fn trace_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> RayHit` and\n  \
         `fn trace_any_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> bool`: `trace`\n  \
         and `trace_any` over the layers `layers` names; `trace(o, d, m)` is `trace_layers(o, d, m, RAY_SOLID)`. With no\n  \
         layer named every ray misses.\n\
         - `fn hit_normal_map(hit: RayHit) -> vec4<f32>`, `fn hit_specular_map(hit: RayHit) -> vec4<f32>`: the resource\n  \
         packs' LabPBR maps at the hit, its sprite's texel at `hit.uv` at full size, as `block_normal_map_level(uv, 0.0)`\n  \
         and `block_specular_map_level(uv, 0.0)` read a drawn surface's; their defaults without maps, and on a miss.\n\
         - In the post stage, `fn block_pbr_maps() -> bool`: whether the resource packs ship LabPBR maps, as the material\n  \
         stage's.\n\n\
         A ray starts at `origin` itself, so move a point off its own surface before tracing from it. What stops it is what\n\
         the surface's own pass draws. In the solid layer it passes through the clear texels of a cut-out texture (leaves,\n\
         plants), as terrain's alpha test does: a texel stops it from alpha 0.5 up. In the translucent layer every texel\n\
         stops it, however faint, except a clear one (alpha below 0.01), which the translucent pass doesn't draw either;\n\
         the hit's `albedo.a` is the texel's alpha and its `translucent` is true. A ray over both layers stops at the\n\
         nearest surface of either: to go on through a translucent one, trace again from just behind the hit. The scene is\n\
         Horizon's: the terrain around the camera, out to `frame.ray_distance` at least, without entities, particles,\n\
         clouds or distant terrain. While\n\
         the game doesn't trace (`frame.has_ray_tracing` is 0) the pack is built and runs all the same, and every ray\n\
         misses. Your files name no ray query of their own (`ray_query`, `acceleration_structure`, `RayDesc`,\n\
         `RayIntersection`, a `rayQuery` function, `enable wgpu_ray_query`): the check refuses them, and an `output`\n\
         named as one of these functions.\n\n\
         In the material stage only `material` traces: `displacement` and the vertex stage's `vertex`, whose files share\n\
         its module, run in vertex shaders, and the check refuses a pack where either reaches a trace function.\n\
         `material` runs for every terrain fragment drawn, so a ray costs more there than in a post pass: keep its rays for\n\
         translucent surfaces (`input.translucent`), which leave no surface for a post pass to light.\n\n",
    );
    out += &table(&RAY_HIT, None);
    out += &format!(
        "Bindings {RAY_SCENE_BINDING} to {} of group {GROUP} hold the scene while the game traces, and in a post pass bindings\n\
         {PBR_NORMALS_BINDING} and {PBR_SPECULAR_BINDING} the LabPBR maps (the material stage has them there already). They are Horizon's, as their\n\
         `horizon_` names say, and change with the renderer.\n\n",
        RAY_SPRITES_BINDING + RAY_SPRITE_ARRAYS - 1
    );
    out
}

/// Kept resources and compute passes, after the post stage.
fn kept() -> String {
    let list = |names: Vec<&str>| {
        let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
        quoted.join(", ")
    };
    let elements = list(Element::ALL.iter().map(|e| e.wgsl()).collect());
    let formats = list(Format::ALL.iter().map(|f| f.wgsl()).collect());
    let mut out = format!(
        "## Kept resources and compute passes\n\n\
         **Checked, not run yet:** `horizon-shader check` takes these keys, and the game refuses a pack that declares a\n\
         `[[resource]]`, a `[[before_world]]` entry or a compute `[[post]]` entry: it needs a later Horizon.\n\n\
         - `[[resource]]`: `name`; `buffer` (bytes) with `element` ({elements}), or `format`\n  \
         ({formats}) with `size = [w, h]` texels or `scale` (1, 0.5 or 0.25 of the render size); `keep`, `\"view\"`\n  \
         (screen history) or `\"world\"` (a cache, fixed `size` only, which needs `[kept] reach`: a teleport further, in\n  \
         blocks, clears it). A texture has this frame's version, `<name>`, and last frame's, `<name>_previous`; a buffer\n  \
         has one. Both names are free WGSL names, as an `output`'s, and no word WGSL reserves.\n\
         - `[[before_world]]`: a compute pass once a frame, before the world draws: `file`, `dispatch` (`[x, y, z]`\n  \
         cells, or `\"pixels\"`, one per texel of the render size), `workgroup = [x, y, z]`, `reads` and `writes`.\n\
         - `[[post]]` with `compute = true` and the same keys: a dispatch in the chain where it stands. It has no `output`\n  \
         or `scale`, isn't the last entry and doesn't advance `previous`. An ordinary pass lists what it `reads`, and its\n  \
         `output` may name a kept texture, which it draws at that texture's size.\n\
         - An option's `resets = \"view\"` clears the view's kept resources when the player changes it, `\"world\"` all\n  \
         of them.\n\n\
         A pass lists every resource it uses: in `reads` this frame's `x` or last frame's `x_previous`, in `writes` (a\n\
         compute pass's) `x`, once. An `output` is its pass's write. Its prelude declares only what it lists, so a name\n\
         it doesn't list is the file's own, and using such a resource is refused as unlisted. The check also refuses a\n\
         compute pass that writes a texture and reads it back (read `_previous`, or a buffer), and a pass that reads its\n\
         own `output`.\n\n\
         A compute file defines `fn compute(input: ComputeInput)`, and Horizon adds the `@compute` entry with the pass's\n\
         workgroup size. Every invocation of every workgroup runs it, so `var<workgroup>` and `workgroupBarrier()`\n\
         reductions stay uniform: one beyond the dispatch has `in_bounds` false and touches no resource. A compute\n\
         `[[post]]` pass's prelude is the post stage's with `ComputeInput` for `PostInput`; a `[[before_world]]` pass has\n\
         the frame, the samplers and the shadow maps (bindings below) with `shadow_position`.\n\n\
         **Not yet, in later steps:** a compute or `[[before_world]]` file doesn't get `trace` or the other ray functions\n\
         under `[ray_tracing]`, and `[material]` can't read kept resources.\n\n"
    );
    out += &table(&COMPUTE_INPUT, None);
    out += &format!(
        "A pass's prelude declares the resources it lists, the pack's resource i at binding {KEPT_BINDING} + 2i of group\n\
         {GROUP} and its `_previous` at {} + 2i: a buffer `var<storage, read_write> x: array<element>` where written,\n\
         `var<storage, read>` where only read (an `atomic<u32>` buffer then reads as `array<u32>`); a texture\n\
         `texture_storage_2d<format, write>` where written, else `texture_2d<f32>` (`texture_2d<u32>` for\n\
         `rgba32uint`, which only a compute pass writes). An `r32float`, `rgba32float` or `rgba32uint` texture is read\n\
         with `textureLoad` only: the check refuses sampling or gathering one, also through a function's argument. Data is data: Horizon doesn't decode, clamp or force the alpha of a kept resource.\n\n\
         `[[before_world]]` resources (bind group {GROUP}):\n\n\
         | Binding | Declaration | Meaning |\n| --- | --- | --- |\n",
        KEPT_BINDING + 1
    );
    for r in StageKind::BeforeWorld.resources() {
        out += &format!("| {} | `{}` | {} |\n", r.binding, r.ty, r.doc);
    }
    out += "\nA kept resource starts as zeros and is zeros again when cleared: a `keep = \"view\"` one when\n\
            `frame.view_epoch` changes, every one when `frame.world_epoch` does (never 0, so a cleared value never\n\
            matches). Both change before the frame that sees them draws, and `frame.reset_reasons` has a bit for each\n\
            reason, a `const` of the prelude (`frame.reset_reasons & REASON_TELEPORT`); only a pack with `[[resource]]`\n\
            has these fields and constants. A world cache keyed by `world_epoch` survives a camera cut. Nothing else\n\
            says the world changed, not the camera's distance or `frame_index`.\n\n\
            | Reason | Bit | Clears | When |\n| --- | --- | --- | --- |\n";
    // What each clears and when, in `REASONS`' order.
    let rows = "both | The pack was loaded or reloaded: its first frame.\n\
                both | Horizon restarted the pack's history (a screenshot sequence's first shot).\n\
                both | Another world or dimension, or its meshes were rebuilt (a render setting).\n\
                both | The camera moved further than `[kept] reach` since last frame.\n\
                view | Over half a second since last frame, as `delta_time` 0.\n\
                view | The render size or the window's changed.\n\
                view | The output turned HDR or SDR.\n\
                its `resets` | The player changed an option marked `resets`.";
    for ((name, bit), row) in REASONS.iter().zip(rows.lines()) {
        out += &format!("| `{name}` | {bit} | {row} |\n");
    }
    out + &format!(
        "\nLimits: {MAX_RESOURCES} resources, {MAX_BEFORE_WORLD} `[[before_world]]` entries, {MAX_COMPUTE_POST} compute\n\
         `[[post]]` entries; {} MiB of buffers and {} KiB of fixed-size textures, both versions counted; each R-relative\n\
         kept texture takes two of the post targets `output`s share; a workgroup of up to {MAX_WORKGROUP:?} and\n\
         {MAX_WORKGROUP_INVOCATIONS} invocations; {MAX_DISPATCH_INVOCATIONS} invocations a dispatch and {MAX_FRAME_INVOCATIONS} a frame over every\n\
         compute pass, `[[before_world]]` and `[[post]]` together, counted as whole workgroups, a `\"pixels\"` dispatch as {}×{} (R isn't known at load).\n\n",
        MAX_BUFFER_BYTES >> 20,
        MAX_FIXED_TEXTURE_BYTES >> 10,
        PIXELS_COUNTED_AS[0],
        PIXELS_COUNTED_AS[1]
    )
}

/// `[material] coverage`, after the paragraph on what a stage is.
const COVERAGE: &str = "\
## `[material] coverage`

Which points of terrain are drawn is the engine's unless the pack says otherwise. Under `coverage = \"engine\"` (the
default) solid and cutout terrain keep Horizon's alpha test on the texture's own alpha (cut below 0.5, except the
leaves Horizon draws opaque), and the alpha `material` returns is ignored there; translucent terrain is cut where the
texture's alpha is below 0.01, and the returned alpha is its opacity, so water and glass still set how see-through
they are. `material`, and every function it calls, may not `discard`: the check refuses it. A pack that cuts its own
holes sets `coverage = \"pack\"`: the returned alpha then drives the alpha test (below 0.5 on solid and cutout
terrain, below 0.01 on translucent) and the stage may `discard`. Normal and parallax shading that keeps the
silhouette needs no `\"pack\"`, and neither do `[vertex]` and `[displacement]`, which move geometry, not coverage.
The stage writes no depth of its own either way.

";

/// The entity stage's notes, after its input: what it returns, and vanilla's own.
const ENTITY_NOTES: &str = "\
`entity` lights what is no block terrain: `input.kind` says which. It returns the same `Surface` as `material` (rgb
linear light, 0 or more and unbounded; alpha its coverage), so a pack writes its lighting once and calls it from both
stages. Around it Horizon keeps vanilla's order: the alpha test on the returned alpha, then the enchantment glint
added on top, then fog. `horizon-shader check` takes the stage; **the game doesn't draw it yet** and refuses a pack
that has it. A pack without `[entity]` is drawn as before, and sees none of this stage's names. Until the game draws
the stage its prelude may still gain names; from then on the rule for new names holds for it too.

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

";

/// The post stage's motion guide: `{binding}` stands for `MOTION_BINDING`.
const MOTION_NOTES: &str = "\
### Motion guide

How each pixel's surface moved since last frame, for temporal passes (TAA, denoisers, light caches) that reproject
`history` or a target of their own. With `[motion]` in the manifest (it has no keys, and needs a `[[post]]` pass)
every post pass's prelude also has the names below; a pack without the key has none of them, and no other stage does.
`horizon-shader check` takes it; **the game doesn't draw it yet** and refuses a pack with `[motion]`.

- `var scene_motion: texture_2d<f32>` (binding {binding}): the engine's objects (z 1 matched, xy the delta; w 1
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

";

/// The reference.
pub fn markdown() -> String {
    let mut out = format!(
        "# Shader API reference ({API})\n\n\
         Generated from `crates/horizon-shader-api` (`HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test it reference::`\n\
         rewrites it; the test fails when it drifts). The API's design and pack format: [shaders.md](shaders.md).\n\n"
    );
    out += RULES;
    out += "A stage is a function your WGSL defines, with the signature below; Horizon puts the prelude (these structs, the\n\
            resources and helpers) before your files and calls the function from its own pass. Your files declare no\n\
            bindings and no entry points, and no names starting with `horizon_`.\n\n";
    out += COVERAGE;
    out += "## Stages\n\n\
            | Stage | In `horizon-shaders.toml` | Your function |\n| --- | --- | --- |\n";
    for kind in StageKind::ALL {
        let key = match kind {
            StageKind::Post => "[[post]]",
            StageKind::Sky => "[sky]",
            StageKind::Material => "[material]",
            StageKind::Vertex => "[vertex]",
            StageKind::Weather => "[weather]",
            StageKind::Tonemap => "[tonemap]",
            StageKind::Entity => "[entity]",
            StageKind::Compute => "[[post]] compute = true",
            StageKind::BeforeWorld => "[[before_world]]",
        };
        out += &format!("| {} | `{key}` | `{}` |\n", kind.name(), kind.signature());
    }
    out += "\n";
    for kind in StageKind::ALL {
        out += &format!("## {}\n\n`{}`\n\n", kind.name(), kind.signature());
        out += &table(kind.input(), None);
        if kind == StageKind::Entity {
            out += ENTITY_NOTES;
        }
        if kind == StageKind::Material {
            out += &table(&SURFACE, None);
            out += "Under `[displacement]` the stage also defines \
                    `fn displacement(input: DisplacementInput) -> f32`, called for each corner of the grid the \
                    faces near the camera are cut into; the corner moves along the face's normal by the depth it \
                    gives at the point's height less the depth at the texture's mean height, pinned at the face's \
                    edges and easing out over the last quarter of the distance.\n\n";
            out += &table(&DISPLACEMENT_INPUT, None);
        }
        out += &format!(
            "Resources (bind group {GROUP}):\n\n| Binding | Declaration | Meaning |\n| --- | --- | --- |\n"
        );
        for r in kind.resources() {
            out += &format!("| {} | `{}` | {} |\n", r.binding, r.ty, r.doc);
        }
        out += "\n";
        if kind == StageKind::Tonemap {
            out += "It is a pure mapping from the scene's light to the output its `frame` describes, run once per pixel of\n\
                    each output, after the post chain and before the camera overlays and the HUD. Today the display is the\n\
                    only output; Horizon may add others for the same frame, such as an SDR screenshot or icon, where `frame`'s\n\
                    display fields describe SDR (`hdr` 0, `headroom` 1, `paper_white` and `peak` 80, `black_level` 0). Its\n\
                    only output is the colour it returns, so nothing carries from one call to the next. A pack without it gets\n\
                    Horizon's: a clip at white in SDR, and in HDR the curve Horizon uses without a pack, which keeps light up\n\
                    to 60% of the peak as it is and rolls it off into the peak above. Either runs once: Horizon doesn't map a pack's\n\
                    result again. Horizon holds the result to 0 to `frame.headroom`, then lifts the shadows onto\n\
                    `frame.black_level`.\n\n";
            continue;
        }
        if kind != StageKind::Sky {
            out += "Helpers:\n\n";
        }
        if kind == StageKind::Post {
            out += "- `fn depth_at(uv: vec2<f32>) -> f32`: the world's depth at `uv`.\n\
                    - `fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32>`: the camera-relative position, in blocks, of\n  \
                    the point at `uv` and `depth`.\n\
                    - `fn surface_at(texel: vec2<i32>) -> SurfaceData`: the solid terrain's surface at a pixel, as the \
                    material stage's `set_surface` left it: `present` (false where the sky, an entity, water, glass or a \
                    particle shows instead), `normal` (world axes), `smoothness` and `data`.\n";
        }
        if kind == StageKind::Material {
            out += "- `fn block_texture(uv: vec2<f32>) -> vec4<f32>`: the surface's block texture at `uv`, anywhere on \
                    the sprite, without the tint.\n\
                    - `fn block_texture_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at mip level `lod`.\n\
                    - `fn block_texture_size() -> vec2<f32>`: its size in texels.\n\
                    - `fn block_texture_lod() -> f32`: the mip level `block_texture` reads at this point.\n";
            out += "- `fn block_pbr_maps() -> bool`: whether the resource packs ship LabPBR maps (`_n`, `_s`).\n\
                    - `fn block_normal_map(uv: vec2<f32>) -> vec4<f32>`: the surface's LabPBR normal map at `uv` (rg the \
                    normal along `input.tangent` and down the texture, b ambient occlusion, a height); (0.5, 0.5, 1, 1) \
                    without one.\n\
                    - `fn block_specular_map(uv: vec2<f32>) -> vec4<f32>`: its LabPBR specular map (r smoothness, g F0 or a \
                    metal, b porosity or subsurface, a emission, 1 none); (0, 0, 0, 0) without one.\n\
                    - `fn block_normal_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`, \
                    `fn block_specular_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at the block texture's mip level `lod`.\n\
                    - `fn block_pbr_size() -> vec2<f32>`: the LabPBR maps' size in texels.\n\
                    - `fn set_surface(normal: vec3<f32>, smoothness: f32, data: f32)`: what the post stage's \
                    `surface_at` reads for this point of solid terrain (`data` the pack's own; the face's normal and 0, 0 unset).\n";
        }
        if kind == StageKind::Entity {
            out += "- `fn block_light_color(position: vec3<f32>) -> vec4<f32>`: the colour of the block light at the \
                    camera-relative `position` (rgb) and how strongly it reaches it (a), as the material stage's.\n";
        }
        if kind != StageKind::Sky {
            out += "- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is\n  \
                    stored in the shadow map, for `textureSampleCompareLevel`.\n\
                    - `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.\n\n";
        }
        if kind == StageKind::Post {
            out += "A `[[post]]` entry with `output = \"<name>\"` writes a target of the pack's own, at its `scale` of the\n\
                    world's size (1, 0.5 or 0.25), instead of the next pass's `previous`; the prelude of every pass declares\n\
                    `var <name>: texture_2d<f32>` (group 3 from binding 32), which holds last frame's until written again (a\n\
                    pass reads a blank one in place of the target it writes).\n\n";
            out += &ray_tracing();
            out += &kept();
            out += &MOTION_NOTES.replace("{binding}", &MOTION_BINDING.to_string());
        }
    }
    out += "## The frame\n\n";
    // Every field, with those only a `[ray_tracing]` or `[[resource]]` pack declares.
    let frame = Frame::declared(true, true);
    out += &table(&frame, Some(&Frame::OFFSETS[..frame.fields.len()]));
    out += "## Anti-aliasing\n\n\
            While a pack's `[[post]]` chain draws the frame, the player's anti-aliasing settings do not run: `taa`, `fxaa`\n\
            and `upscaling` in `horizon-graphics.txt` have no effect, and the frame's projection is never jittered, because the\n\
            last post pass draws the picture itself and Horizon's own resolve, which does TAA, FXAA and TAA upscaling, is the\n\
            step it replaces. The world is still rendered at `resolutionScale` before the pack reads it.\n\n\
            A pack that wants smooth edges draws them itself, in a post pass: a spatial filter (FXAA, SMAA and the like) works on\n\
            `previous` as it is. A temporal pass cannot do what TAA does here, even though `history` and\n\
            `frame.previous_view_projection` give it the last frame: nothing moves the camera by a fraction of a texel from one\n\
            frame to the next, so a still camera gives it the same picture twice and there is no sub-pixel detail to gather.\n\n\
            Without a pack the settings apply as the player set them.\n\n";
    out += &format!(
        "## Options\n\n\
         Each `[[options]]` entry sets the WGSL `override` of the same name when the pipelines are built:\n\n\
         | `type` | Keys | The `override` |\n| --- | --- | --- |\n\
         | `bool` | `default` | `override <id>: bool` |\n\
         | `float` | `default`, `min`, `max`, `step` | `override <id>: f32` |\n\
         | `int` | `default`, `min`, `max` | `override <id>: i32` |\n\
         | `choice` | `values`, `default` (one of them) | `override <id>: u32`, the value's index |\n\n\
         ## Limits\n\n\
         - {MAX_POST_PASSES} post passes, {MAX_OPTIONS} options, `budget.gpu_ms` up to {MAX_GPU_MS}.\n\
         - {} KiB per WGSL file, {} MiB per pack.\n\
         - Per stage: {MAX_EXPRESSIONS} naga expressions over all its functions, {MAX_TEXTURE_READS} texture reads (samples,\n  \
         gathers and loads, the prelude's included).\n\n\
         ## The check\n\n\
         A pack runs only after it passes `horizon_shader_api::check` (the game when it loads a pack, the launcher before it\n\
         installs one, the registry's CI): each stage is parsed and validated by naga, checked against its signature and\n\
         the rules above, and translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL) with its options at their\n\
         defaults, minimums and maximums. Every option must be an `override` of its type in some stage; an `override`\n\
         that is no option needs a default.\n",
        MAX_WGSL >> 10,
        MAX_TOTAL >> 20
    );
    out
}
