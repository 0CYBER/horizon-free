//! docs/api/shaders-reference-0.2.md, generated from the stages and inputs this crate defines
//! (`tests/reference.rs` fails when the file drifts).
use crate::check::{MAX_EXPRESSIONS, MAX_TEXTURE_READS};
use crate::inputs::{
    COMPUTE_INPUT, DISPLACEMENT_INPUT, Frame, LIT_TINT, MATERIAL_INPUT, RAY_HIT, SHADED_TINT,
    SURFACE, Struct,
};
use crate::kept::{
    Element, Format, MAX_BEFORE_WORLD, MAX_BUFFER_BYTES, MAX_COMPUTE_POST,
    MAX_DISPATCH_INVOCATIONS, MAX_FIXED_TEXTURE_BYTES, MAX_FRAME_INVOCATIONS, MAX_RESOURCES,
    MAX_WORKGROUP, MAX_WORKGROUP_INVOCATIONS, PIXELS_COUNTED_AS, REASONS,
};
use crate::manifest::{
    API, MAX_GPU_MS, MAX_OPTIONS, MAX_POST_PASSES, MAX_POST_TARGETS, POST_TARGET_BUDGET,
};
use crate::package::{MAX_TOTAL, MAX_WGSL};
use crate::stages::{
    GROUP, KEPT_BINDING, MOTION_BINDING, PBR_NORMALS_BINDING, PBR_SPECULAR_BINDING,
    RAY_ENTITY_RETAINED_BINDING, RAY_ENTITY_STORE_BINDING, RAY_LIGHTS_BINDING, RAY_LOOKS_BINDING,
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

/// `table` with `SHADED_TINT` as the last row: the struct ends with it in a pack that names it.
fn shaded_table(s: &Struct) -> String {
    input_table(s, true)
}

/// `table` with the rows of the fields a pack's files may add (`inputs::light_source`,
/// `LIT_TINT` in `MaterialInput`, then `SHADED_TINT` where `shaded`), in the order the struct
/// has them.
fn input_table(s: &Struct, shaded: bool) -> String {
    let mut out = table(s, None);
    out.pop();
    let lit = (s.name == MATERIAL_INPUT.name).then_some(&LIT_TINT);
    let shaded = shaded.then_some(&SHADED_TINT);
    for f in crate::inputs::light_source(s)
        .iter()
        .chain(lit)
        .chain(shaded)
    {
        out += &format!("| `{}` | `{}` | {} |\n", f.name, f.ty, one_line(f.doc));
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
**Preview** ([stability.md](stability.md)), final: `horizon-shader check` takes 0.2 packs, the registry lists them and
the game draws them. The game alone refuses two keys for now, which a later Horizon draws: `[frame] primaries =
\"bt2020\"` and `[motion]`. What this page says stays; a change that breaks a 0.2 pack only comes with a new line
(0.3), after a deprecation.
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
  the pack's targets and kept resources, `opaque_color`, `reflection`, `shadow_color`).
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

## `[frame] fog`

`fog = \"engine\"` (the default): Horizon mixes vanilla's fog over what the material and entity stages return. With
`fog = \"pack\"` the pack fogs what its stages draw: Horizon skips its fog there, and scales the enchantment glint it
adds after `entity` by `1 - vanilla_fog(position)`, as vanilla's fog over it would. What Horizon draws itself keeps
vanilla's fog: items, particles, clouds, the sun and moon, the hand, block outlines, beams, the water mirror's terrain,
and the fade of newly loaded terrain toward `frame.fog_color`. The sky, weather and post stages get no fog from Horizon
either way. Every pack has `frame.fog_environment` and `frame.fog_render` (below) in every stage; a pack with the key
also has, in every stage:

- `fn vanilla_fog(position: vec3<f32>) -> f32`: how much of `frame.fog_color` vanilla mixes over a surface at the
  camera-relative `position`, 0..1, under water, lava, powder snow, blindness and darkness too: the larger of
  the environment's fog along `length(position)` and the render distance's along
  `max(length(position.xz), abs(position.y))`, each 0 before its start, 1 from its end and linear between. With the
  Fog option off it is 0, except under water, lava, powder snow, those effects and a boss's fog.

Vanilla mixes in its gamma, under either `[frame] blend`:
`pow(mix(pow(c, vec3(1.0 / 2.2)), pow(frame.fog_color, vec3(1.0 / 2.2)), vanilla_fog(input.position)), vec3(2.2))` on
the lit rgb, alpha kept. Horizon's own takes the distance at each corner and interpolates it, `vanilla_fog` at each
point, so a pack's copy may round a step apart on large faces. Mixing the linear values instead is fog as light
scatters, a little denser in the middle tones. A post pass fogs what the stages don't draw from `scene_depth`
(`vanilla_fog(view_position(uv, depth))`), and traced hits from `hit.position`, which Horizon never fogs.
`[material] fog`, from before, is gone: the check refuses it and names `[frame] fog`.

";

/// `[ray_tracing]`, at the end of the post stage.
fn ray_tracing() -> String {
    let mut out = String::from(
        "### Ray tracing\n\n\
         With `[ray_tracing]` in the manifest (it needs a `[[post]]` pass, a `[[before_world]]` pass, a `[material]` or an\n\
         `[entity]` stage; its one key, `lights`, is below), post passes get `RayHit` and the functions and constants\n\
         below. So do the material, entity, compute and `[[before_world]]` stages whose files name one of the four trace\n\
         functions (or, with `lights = true`, a light one). Every stage of the pack has `frame.has_ray_tracing` and\n\
         `frame.ray_distance`. A pack without the key has none of these. A material, entity, compute or `[[before_world]]`\n\
         stage that names no trace function (nor, with `lights = true`, a light one) is built as without it.\n\n\
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
         - `const RAY_ENTITY: u32`: a third layer, only in a pack whose files name it, anywhere outside a comment (dead\n  \
         code too): the entities within the scene's reach, in view or not (behind the camera, hidden), which are mobs,\n  \
         players, armour stands, item frames, vehicles, projectiles, falling blocks, block displays, the block entities\n  \
         at rest that the block entity cache keeps (on in every graphics preset: closed chests, shulker boxes, bells,\n  \
         decorated pots, banners, within vanilla's 64 blocks) in view or not, and the other block entities the camera\n  \
         draws (a chest while it opens). Such a pack's `RayHit` also has `entity`. Not in it: particles, text, the hand,\n  \
         the camera's own body in first person, translucent items, additive glows, beams and portals, and other block\n  \
         entities out of view. `trace` and `trace_any` never read it.\n\
         - `fn trace_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> RayHit` and\n  \
         `fn trace_any_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> bool`: `trace`\n  \
         and `trace_any` over the layers `layers` names; `trace(o, d, m)` is `trace_layers(o, d, m, RAY_SOLID)`. With no\n  \
         layer named every ray misses. Several layers cost one traversal: the layers are the ray query's mask, not a\n  \
         query each.\n\
         - `fn hit_normal_map(hit: RayHit) -> vec4<f32>`, `fn hit_specular_map(hit: RayHit) -> vec4<f32>`: the resource\n  \
         packs' LabPBR maps at the hit, its sprite's texel at `hit.uv` at full size, as `block_normal_map_level(uv, 0.0)`\n  \
         and `block_specular_map_level(uv, 0.0)` read a drawn surface's; their defaults without maps, and on a miss.\n\
         - In the post, entity, compute and `[[before_world]]` stages, `fn block_pbr_maps() -> bool`: whether the resource packs\n  \
         ship LabPBR maps, as the material stage's.\n\
         - The light sources, only with `lights = true` under `[ray_tracing]` (off by default: without it these five names\n  \
         are free for the pack's own), and only in a stage whose files name one of them outside a comment, which alone\n  \
         binds them. `struct RayLight`: `position: vec3<f32>`, the centre of its block, camera-relative from the player's\n  \
         camera (`frame.camera_position`) in every pass, as `trace`'s `origin` and `view_position` are (the water mirror\n  \
         is drawn by Horizon's own terrain shader, and the shadow map with the vertex stage, which can't reach the light\n  \
         functions: no pass reads them from another camera); `level: f32`,\n  \
         vanilla's light level of the block, 1 to 15 (a terrain hit's `RayHit.emission` times 15); `color: vec3<f32>`,\n  \
         its `[[lights]]` colour decoded to linear as `block_light_color`'s is, white for a block the pack gives none;\n  \
         `material: u32`, its `[[materials]]` id, the one `RayHit.material` reports on the first face of its model whose\n  \
         sprite has one, 0 where none does; it is taken from one fixed model of the block state, so where a resource pack\n  \
         gives the state weighted models with different materials it can differ from the `RayHit.material` of that\n  \
         block. `fn ray_light_count() -> u32` and `fn ray_light(i: u32) -> RayLight`: the blocks that give off light\n  \
         in the sections the grid or 15 blocks around it touch (so up to a section past where their light reaches it),\n  \
         the nearest the camera first, at most 1024; `ray_light` gives level 0 past the count.\n  \
         Fluids are left out: lava is not listed, so lava lakes don't light through `ray_lights_near` (they would fill\n  \
         every cell nearby and push the torches out; a ray meets the lava's surface anyway, with its `emission`).\n  \
         `fn ray_lights_near(position: vec3<f32>) -> vec2<u32>`: the sources whose light reaches some part of the cell of\n  \
         a camera-relative point, as the first and the count of the indices `fn ray_light_at(i: u32) -> u32` reads;\n  \
         none outside the grid. `ray_light_at` gives `0xffffffffu`, which `ray_light` reads as level 0, for an `i` past\n  \
         its cell's count. Both are candidate sets, not a verdict: a source listed may not reach the point itself, so\n  \
         the pack's own falloff (and a trace) decides what each gives. A cell's order is its strength at the cell's\n  \
         nearest point, not at the point: a pass that tests only its first few shows the cells' edges. The list is\n  \
         the data, and the grid a convenience built from it with fixed choices (8-block cells, 16 a side around the\n  \
         camera, at most 24 a cell, ranked by vanilla's falloff, level minus distance, at the cell's nearest point): a\n  \
         pack that wants others reads the list and builds its own grid into a kept buffer in a compute or\n  \
         `[[before_world]]` pass. Both are rebuilt off the render thread when the camera crosses a 16-block step, and\n  \
         at most every 100 ms when a source block changes, and reach the passes a frame or two later; their positions\n  \
         stay relative to the current camera meanwhile. The list only says where sources are: trace towards one (`trace_any`) to know whether\n  \
         its light gets through. A full glowing cube (glowstone, a sea lantern) is in the scene itself, so a ray aimed\n  \
         at its centre is stopped by the source: end the ray where it enters the source's block (as\n  \
         `packs/traced_lights` does), or take a hit within the block as the source.\n\n\
         A ray starts at `origin` itself, so move a point off its own surface before tracing from it. What stops it is what\n\
         the surface's own pass draws. In the solid layer it passes through the clear texels of a cut-out texture (leaves,\n\
         plants), as terrain's alpha test does: a texel stops it from alpha 0.5 up. In the translucent layer every texel\n\
         stops it, however faint, except a clear one (alpha below 0.01), which the translucent pass doesn't draw either;\n\
         the hit's `albedo.a` is the texel's alpha and its `translucent` is true. In the entity layer a texel stops it where\n\
         vanilla's entity alpha test keeps the pixel, `(texel * tint).a` 0.1 or more (on the dying dragon's dissolve,\n\
         vanilla's mask decides), from either side of the face. A ray over several layers stops at the nearest surface of\n\
         any: to go on through a translucent one, trace again from just behind the hit. No face is culled, and\n\
         `hit.normal` is turned toward the ray's origin; `hit.back` says the ray met the face from behind (see it for faces\n\
         drawn on both sides): a ray that starts inside a full block hits that block's drawn faces from behind. The\n\
         scene is Horizon's: the terrain around the camera, out to `frame.ray_distance` at least, without particles, clouds or distant terrain,\n\
         and with entities only in `RAY_ENTITY`. While\n\
         the game doesn't trace (`frame.has_ray_tracing` is 0) the pack is built and runs all the same, and every ray\n\
         misses. Your files name no ray query of their own (`ray_query`, `acceleration_structure`, `RayDesc`,\n\
         `RayIntersection`, a `rayQuery` function, `enable wgpu_ray_query`): the check refuses them, and an `output`\n\
         named as one of these functions.\n\n\
         In the material stage only `material` traces: `displacement` and the vertex stage's `vertex`, whose files share\n\
         its module, run in vertex shaders, and the check refuses a pack where either reaches a trace function.\n\
         A ray in `material` is paid for every terrain fragment drawn, overdraw included; a ray in a post pass once a\n\
         pixel. But a post pass knows only the solid terrain's surface (`surface_at` isn't `present` behind water,\n\
         glass or entities) and has no albedo, so direct light that must be right there is traced in `material`. With\n\
         `[material] reads`, a pack can also trace in a compute or `[[before_world]]` pass and apply the result in\n\
         `material`.\n\n\
         In the entity stage `entity` traces as `material` does, in the world's colour pass only (the shadow map's\n\
         entities don't run the stage). A ray it casts with `RAY_ENTITY` in its layers can hit the entity it starts on:\n\
         move its origin off the surface, as from terrain.\n\n",
    );
    out += &shaded_table(&RAY_HIT);
    out += &format!(
        "Bindings {RAY_SCENE_BINDING} to {} of group {GROUP} hold the scene while the game traces ({RAY_ENTITY_STORE_BINDING} to\n\
         {RAY_ENTITY_RETAINED_BINDING} the entity layer, for a pack that names it, {RAY_LIGHTS_BINDING} the light sources, for a pass that\n\
         names them under `lights = true`), and in a post, compute or `[[before_world]]` pass bindings\n\
         {PBR_NORMALS_BINDING} and {PBR_SPECULAR_BINDING} the LabPBR maps (the material and entity stages have them there already).\n\
         Binding {RAY_LOOKS_BINDING} holds terrain hit looks for a stage that reads `RayHit.tint`, `shade` or `ao` from its entry point.\n\
         These bindings are Horizon's, as their `horizon_` names say, and change with the renderer.\n\n",
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
         reductions stay uniform: one beyond the dispatch has `in_bounds` false and touches no resource. WGSL's\n\
         `storageBarrier()` is allowed too: it orders one workgroup's accesses to its kept buffers, as\n\
         `workgroupBarrier()` does `var<workgroup>`; neither orders different workgroups, and the next pass sees all of a\n\
         pass's writes. A compute\n\
         `[[post]]` pass's prelude is the post stage's with `ComputeInput` for `PostInput`; a `[[before_world]]` pass has\n\
         the frame, the samplers and the shadow maps (bindings below) with `shadow_position`. Under `[ray_tracing]` either\n\
         may trace (see Ray tracing); a `[[before_world]]` pass traces this frame's scene, built before it runs.\n\n\
         `[material] reads` lists the kept resources the terrain reads (no `writes`): `x` for this frame's version of a\n\
         texture a `[[before_world]]` pass writes, which the terrain sees in the same frame, `x_previous` for last\n\
         frame's, and a buffer as it is. A texture only `[[post]]` passes write is written after the terrain draws, so\n\
         the check refuses its `x` there: read `x_previous`. Its declarations are in the material's module at the same\n\
         bindings, for `material`'s fragments only: the check refuses `displacement` reaching one, and the `[vertex]`\n\
         file naming one, as both also draw the shadow map, which gets no kept resources. On a frame that clears a\n\
         resource the terrain reads zeros, or what a `[[before_world]]` pass wrote since.\n\n\
         `[entity] reads` and `[sky] reads` follow the same rules: the entities and the sky draw after `[[before_world]]`\n\
         and before `[[post]]`, as the terrain does, and read the same versions. Their declarations are in the stage's\n\
         module, for its fragments only; the shadow map's entities are Horizon's own draw and read none.\n\n"
    );
    out += &table(&COMPUTE_INPUT, None);
    out += &format!(
        "A pass's prelude declares the resources it lists, the pack's resource i at binding {KEPT_BINDING} + 2i of group\n\
         {GROUP} and its `_previous` at {} + 2i: a buffer `var<storage, read_write> x: array<element>` where written,\n\
         `var<storage, read>` where only read (an `atomic<u32>` buffer then reads as `array<u32>`); a texture\n\
         `texture_storage_2d<format, write>` where written, else `texture_2d<f32>` (`texture_2d<u32>` for\n\
         `rgba32uint` and `r32uint`, which only a compute pass writes). An `r32float`, `rgba32float`, `rgba32uint` or\n\
         `r32uint` texture is read with `textureLoad` only: the check refuses sampling or gathering one, also through a function's argument. Data is data: Horizon doesn't decode, clamp or force the alpha of a kept resource.\n\n\
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
         `[[post]]` entries; {} MiB of buffers and {} MiB of fixed-size textures, both versions counted; each R-relative\n\
         kept texture takes its two versions' size of the post targets' budget (Limits); a workgroup of up to {MAX_WORKGROUP:?} and\n\
         {MAX_WORKGROUP_INVOCATIONS} invocations; {MAX_DISPATCH_INVOCATIONS} invocations a dispatch and {MAX_FRAME_INVOCATIONS} a frame over every\n\
         compute pass, `[[before_world]]` and `[[post]]` together, counted as whole workgroups, a `\"pixels\"` dispatch as {}×{} (R isn't known at load).\n\n",
        MAX_BUFFER_BYTES >> 20,
        MAX_FIXED_TEXTURE_BYTES >> 20,
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

## `[entity] coverage`

The same rule for what the entity stage draws, without translucent terrain's exception. Under `coverage = \"engine\"`
(the default) Horizon keeps its own coverage of each entity: the texture's alpha times the layer's opacity (the
vertex alpha: a mob's translucent layer, a fading warden) is cut below 0.1 and is the alpha the layer blends by;
dissolve masks, back-face culling and the rest of Horizon's cuts stay Horizon's. The alpha `entity` returns is
ignored, and `entity`, and every function it calls, may not `discard`: the check refuses it. A pack that cuts or
fades entities itself sets `coverage = \"pack\"`: the returned alpha is then cut below 0.1 and blends, and the stage
may `discard`. Horizon's own cuts run under either.

";

/// The entity stage's notes, after its input: what it returns, and vanilla's own.
const ENTITY_NOTES: &str = "\
`entity` lights what is no block terrain: `input.kind` says which. It returns the same `Surface` as `material` (rgb
linear light, 0 or more and unbounded; its alpha read only under `[entity] coverage = \"pack\"`), so a pack writes
its lighting once and calls it from both stages. Around it Horizon keeps vanilla's order: the alpha test (on the
texture and the layer's opacity, or under `\"pack\"` on the returned alpha), then the enchantment glint added on top,
then fog (the pack's own under `[frame] fog = \"pack\"`). A pack without `[entity]` is drawn as before, and sees
none of this stage's names; the rule for new names holds for the stage as for the others.

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

";

/// The post stage's motion guide: `{binding}` stands for `MOTION_BINDING`.
const MOTION_NOTES: &str = "\
### Motion guide

How each pixel's surface moved since last frame, for temporal passes (TAA, denoisers, light caches) that reproject a
kept resource of their own (`<name>_previous`). With `[motion]` in the manifest (it has no keys, and needs a
`[[post]]` pass) every post pass's prelude also has the names below; a pack without the key has none of them, and no
other stage does. `horizon-shader check` takes it; **the game doesn't draw it yet** and refuses a pack with
`[motion]`.

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
        out += &input_table(
            kind.input(),
            matches!(kind, StageKind::Material | StageKind::Vertex),
        );
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
                    the point at `uv` and `depth`. With view bob or nausea spin active, the ray through this point need not\n  \
                    pass through `(0, 0, 0)`. Get its direction from two positions at the same `uv` and different depths;\n  \
                    for example, `normalize(view_position(uv, depth_far) - view_position(uv, depth_near))`, where\n  \
                    `depth_far < depth_near` because depth is reversed.\n\
\n  \
                    For a perspective projection's finite optical origin, column 2 of `frame.projection_inverse` gives\n  \
                    that origin as a homogeneous view-space point. Transform it to camera-relative world axes:\n\
\n  \
                    ```wgsl\n  \
                    let optical_h = frame.projection_inverse[2];\n  \
                    if optical_h.w != 0.0 {\n    \
                        let optical_view = optical_h.xyz / optical_h.w;\n    \
                        let optical_origin = (frame.view_inverse * vec4<f32>(optical_view, 0.0)).xyz;\n  \
                    }\n  \
                    ```\n\
\n  \
                    An orthographic projection has no finite optical origin (`optical_h.w == 0.0`); do not divide by zero.\n\
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
    // Every field, with those only a `[ray_tracing]`, `[[resource]]` or `[frame] fog = "pack"`
    // pack declares.
    out += &table(&Frame::WGSL, Some(Frame::OFFSETS));
    out += "## Anti-aliasing\n\n\
            While a pack's `[[post]]` chain draws the frame, the player's anti-aliasing settings do not run: `taa`, `fxaa`\n\
            and `upscaling` in `horizon-graphics.txt` have no effect, and the frame's projection is never jittered, because the\n\
            last post pass draws the picture itself and Horizon's own resolve, which does TAA, FXAA and TAA upscaling, is the\n\
            step it replaces. The world is still rendered at `resolutionScale` before the pack reads it.\n\n\
            A pack that wants smooth edges draws them itself, in a post pass: a spatial filter (FXAA, SMAA and the like) works on\n\
            `previous` as it is. A temporal pass cannot do what TAA does here, even though a kept resource and\n\
            `frame.previous_view_projection` give it the last frame: nothing moves the camera by a fraction of a texel from one\n\
            frame to the next, so a still camera gives it the same picture twice and there is no sub-pixel detail to gather.\n\n\
            A pack without `[[post]]` whose frame holds gamma light (not `blend = \"linear\"` on the float frame) gets the\n\
            player's FXAA or TAA: with `fxaa` or `taa` on, Horizon anti-aliases the world, FXAA at its size, in the frame's\n\
            encoding, before the pack's `[tonemap]` (or Horizon's curve) reads it. TAA jitters the world's projection by\n\
            `frame.jitter`, and its history starts over when the pack is loaded or rebuilt and when the display's values\n\
            change. Where the pack has no `[tonemap]`, or one that never reads `previous`, TAA upscaling applies too: the world\n\
            draws at the `upscaling` ratio's share of the scaled window, and TAA rebuilds it at the scaled window's size, then\n\
            sharpens it there, in the frame's encoding, before the decode and the tone curve. `frame.resolution` stays the size\n\
            the world draws at, and the tonemap's `input.color` samples the picture TAA rebuilt. A `[tonemap]` that reads\n\
            `previous` keeps the picture at the world's size: TAA runs for it without its upscaling and sharpening. Either way\n\
            `resolutionScale` still reaches the window through the tone curve's read. Under any other pack TAA is off.\n\n\
            A pack must never write an infinite value into the world's frame: TAA's history would carry it.\n\n\
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
         - Post targets: {MAX_POST_TARGETS} `output`s, and {POST_TARGET_BUDGET} render-size `rgba16float` textures of memory with the\n  \
         R-relative kept textures: an `output` takes its `scale`² (1, 0.25 or 0.0625), an R-relative kept texture its two\n  \
         versions at their size, `2 × scale² × bytes a texel / 8` (`rgba16float` at scale 0.5 takes 0.5, `r32uint` at\n  \
         scale 1 takes 1, `rgba32float` at scale 1 takes 4).\n\
         - {} KiB per WGSL file, {} MiB per pack.\n\
         - Per stage: {MAX_EXPRESSIONS} naga expressions over all its functions, {MAX_TEXTURE_READS} texture reads (samples,\n  \
         gathers and loads, the prelude's included).\n\n\
         ## The check\n\n\
         A pack runs only after it passes `horizon_shader_api::check` (the game when it loads a pack, the launcher before it\n\
         installs one, the registry's CI): each stage is parsed and validated by naga, checked against its signature and\n\
         the rules above, and translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL) with its options at their\n\
         defaults, minimums and maximums. Every option must be an `override` of its type in some stage; an `override`\n\
         that is no option needs a default.\n\n\
         ## Tables and constants\n\n\
         On Vulkan, a read of a module-level `const` array with an index known only at run time first stores the whole array\n\
         to a temporary, again on each turn of a loop, which can cost milliseconds a frame. Declare such a table\n\
         `var<private> NAME: array<T, N> = array<T, N>(...);`, above all where a loop reads it; a constant index avoids the\n\
         store either way.\n\n\
         In a material, vertex or entity stage, the game can refuse some uses of a `let` bound to a constant vector, matrix or\n\
         struct whose components a later statement reads (`let named = TABLE[1];`, then `max(named.x, named.y)`) when it builds\n\
         the pipeline (\"... can't be introduced - it's already in scope\"). It is a naga limitation (gfx-rs/wgpu#7793, #9002)\n\
         that may go away. Write `const named = TABLE[1];` or `var named = TABLE[1];` instead, or read `TABLE[1].x` in place.\n\n\
         ## Written ray calls\n\n\
         On Vulkan, a driver can build the ray traversal anew for every place a stage's code calls a function that traces\n\
         (`trace` and its kin, or a function of yours that calls them), and each such place can then cost time on every\n\
         pixel, whether the pixel takes it or not. On an RTX 5070 Ti at 2560x1440, twelve more written calls that no\n\
         pixel took added 0.6 to 1.5 ms a frame, more the more the called function traces (0.05 to 0.13 ms a call), and\n\
         putting the one ray call inside a loop that ran once added 0.3 to 0.5 ms over the bare call. Write the call once\n\
         and choose its arguments before it (the origin, the direction, the layers, the distance) instead of writing it\n\
         in each branch.\n",
        MAX_WGSL >> 10,
        MAX_TOTAL >> 20
    );
    out
}
