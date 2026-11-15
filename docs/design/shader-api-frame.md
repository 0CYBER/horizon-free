# The shader API 0.2 frame: phases, coverage and room to grow

Design for [#16126](https://github.com/horizon-rust/horizon/issues/16126), after approval from the HDR lead.
Shader API 0.2 is experimental. Change its two defaults now, with no new API line or compatibility route.
This note specifies the intended contract. It does not claim that the executor already implements it.

**Status: proposed.** Written against main plus the open HDR work (the entity stage #15942 and the
`horizon_hdr` pack #16113). The tracing extensions are proposals in
[#15624](https://github.com/horizon-rust/horizon/issues/15624) and
[#16011](https://github.com/horizon-rust/horizon/issues/16011).

## The frame's order (in now)

**World stages → pack compute/post at render resolution → engine resolve → frame generation → per-output
composition and `[tonemap]` → camera overlays, HUD and menus → engine display encoding.**

Frame generation comes **before** tone mapping. Interpolating the mapped picture would contradict
`hdr-passes.md`. It produces another scene, which takes the same output route as a real scene.

| Phase | What runs | Size and ownership |
| --- | --- | --- |
| Begin real frame | Freeze output, camera, sizes, jitter and reset state. Prepare ray scene and requested shadow/mirror inputs. | One snapshot per view. Engine owns frame encoding and draw lists. |
| Before world, when requested later | Pack compute that updates a world-space cache from this frame's ray scene and previous cache. | Declared dispatch; no current screen depth yet. |
| World | `[sky]`; engine celestial draws; terrain `[vertex]`/`[material]`; `[entity]`; translucent layers, particles, clouds and `[weather]`, at their engine draw points. | Actual render size R, including Resolution Scale and the selected upscaler's input ratio. Preserve engine ordering, fog, depth and blending. |
| Scene chain | Decode the stored frame once if needed; run `[[post]]` in order. Later, interleave declared compute dispatches. | Linear working-space light. Ordinary chain links are R; named targets may be smaller. No hand or HUD. |
| Engine resolve | One temporal backend through #12926 **or** spatial FXAA/copy; spatial resampling as needed; CAS where selected. | R → resolve size → output size O. The intermediate resolve size can differ from O under Resolution Scale. |
| Frame generation | Interpolate the resolved, pack-processed scene using depth, motion and optical flow. | O, float light, before hand and HUD. No pack state advances here. |
| Per-output composition | Draw the separate hand layer; entity outlines and resource-pack post effects; call the pack's `[tonemap]` or the engine curve once. | O for that output. Scene effects receive actual depth plus its size. Preserve hand-depth semantics separately. |
| Display composition | Camera overlays; HUD/menus on their own layer; menu blur; black lift, gamut conversion, transfer encoding and output dither. | Engine-owned. `[tonemap]` returns linear display light; it never writes PQ or applies black lift. |

TAA and temporal upscaling are alternative backends, not consecutive temporal filters. FXAA is the spatial
alternative, not an automatic extra pass after TAA. CAS remains a separate selected spatial operation.
No-pack SDR keeps its current path and byte identity; this order changes native 0.2 packs.

The hand needs an explicit transition. Today it draws **after** a native pack's tone map and clears depth.
Keep that legacy hand route until a follow-up to #14583 supplies a separate hand colour/depth layer and its
projection. Then compose its scene-linear light after generation and before per-output effects/mapping.
Do not move today's hand into the world target or pass its cleared depth off as world depth.
Until that follow-up, whole-scene exposure does not include the hand; say so in the reference.

Resource effects must precede the curve on both native and no-pack routes. Their depth input is the retained
world depth at R, explicitly resampled/reconstructed for O, plus the separate hand information where needed.
World colour effects run once on each presented output, without advancing real-frame pack histories.
This fixes the present native route's early tone map and missing main-depth input; it is #16126 step 12.

## Coordinates and temporal ownership (in now)

Add fields to the existing input structs. Do not add constructors or overload `Frame.resolution` per pass.

| Input | Meaning |
| --- | --- |
| `frame.resolution` | Actual R, constant through the real frame. Never the window or a reduced bloom target. |
| `frame.output_resolution` | O of the output being mapped; the view's drawable size during real-frame scene work. A capture can have another O. |
| `PostInput.target_size` | This pass's attachment size, in pixels. `pixel` is its pixel centre, including on the last pass. |
| `PostInput.input_size` | Dimensions of `previous`. `color` samples `previous` at `uv`, bilinearly as today. |
| `PostInput.uv` / `pixel` | Top-left origin; `uv = pixel / target_size`. Final ordinary post target is R, never implicitly O. |
| `scene_color`, `scene_depth`, `scene_surface` | The original scene at R. A named-target write does not replace these or advance `previous`. |
| `depth_at`, `view_position`, `surface_at` | Depth reconstruction uses this frame's raster projection. `surface_at` takes an R texel, not `input.pixel` from a smaller target. Use `floor(uv * frame.resolution)`. |
| `TonemapInput.pixel`, new `output_size` | Pixel centre and dimensions of this output, O. `previous` is the resolved/composed pre-curve scene at O; `input.color` samples it. |

Declare AA ownership as `[frame] antialiasing = "engine"` (default), `"spatial"`, or `"temporal"`.
`spatial` and `temporal` mean the **pack** does AA: skip engine TAA/FXAA and its temporal upscaler.
Resolution Scale, spatial resampling to O and the separate sharpening setting remain available.
`temporal` requests the engine's sub-pixel sequence even with engine TAA off. `spatial` requests no jitter.
Only one owner selects jitter and advances it, once per real frame. A denoiser that does not replace AA keeps
`engine`; having temporal storage alone does not make a pack an AA implementation.
Pack-owned temporal **upscaling** to O is additive later, with an explicit resolve stage through the same seam;
ordinary `[[post]]` cannot silently change the chain's output size or consume the player's upscale ratio.

`Frame.projection` and `projection_inverse` become the actual jittered raster pair. Add
`unjittered_projection`, its inverse, `previous_unjittered_view_projection`, `jitter_pixels`,
`previous_jitter_pixels` and `history_valid`. `previous_view_projection` is the previous raster pair's product.
Jitter is in R pixels, x right, y down. The clip translation is `(2*j.x/R.x, -2*j.y/R.y)`.
World wrappers, sky rays, depth reconstruction and reprojection read this same snapshot; stop independently
reconstructing an unjittered camera in `NativeFrame`. Keep `camera_motion` in blocks, current camera minus previous.
Reproject a static current camera-relative point `p` with the previous matrix at `p + camera_motion`.
Unjittered motion excludes both frames' jitter; add their sampling offsets only when addressing raster histories.

`frame_index`, `delta_time`, compute dispatches, named-target swaps and histories advance once per **real** frame.
Generated frames and an extra SDR capture must not run an accumulating post chain again.
Keep today's `history` meaning: the previous real frame's input to the last ordinary post pass, at R, black when
invalid. It stays black when no ordinary pass precedes the last one (a single pass, or only named-target passes
before it), as today; a pack that needs its own previous frame keeps a named target or, later, a typed kept target.
Do not redefine it to mean resolved colour. Named-target reads retain their existing ordered-write behaviour.
Current post entries force alpha to 1, including named outputs: they cannot yet store a distance in alpha.
Typed data targets below remove that limit without changing colour-target alpha semantics.

Invalidate screen histories on load/reload, world/dimension change, camera cut or teleport, pause gap, R/O change,
AA-owner/backend change, encoding/primaries change and effective-pack change. Publish invalidity before drawing.
On reset, previous matrices equal current ones, previous jitter equals current jitter, and camera motion is zero.
Display-only changes invalidate histories that used display values; scene-only histories need not be dropped.

## Output-resolution effects (`[tonemap]` now; output chains later)

`[tonemap]` already runs at O, samples neighbours of `previous`, and can read declared PNG textures.
Use it for a small lens-dirt composite or film grain; the function performs the effect and the curve together.
Add dirt in scene light before the curve, or grain to mapped linear display light within the same invocation.
Use `output_size`, not R, for pixel frequency. Keep the function stateless and bounded by `frame.headroom`.
A pack supplying it also supplies the curve; it does not receive another engine tone map afterwards.
Named bloom/history targets are not currently declared in its prelude. Add read-only target imports under an
explicit `[tonemap] inputs = [...]` key when needed; do not expose every post name globally.
An output-size multipass chain is additive later if one function is insufficient. It must be stateless, run per
presented output, follow generation and precede encoding. Never move ordinary temporal `[[post]]` after resolve.

## Coverage (in now: the second breaking change)

Use `[material] coverage = "engine"` by default; `coverage = "pack"` is the exception.
This is an enforced rule, not a promise inferred from the final alpha or from source-text searches.

| Mode | Solid/cutout | Translucent | Optimizations |
| --- | --- | --- | --- |
| `engine` | Engine tests original sampled alpha, including force-opaque leaves and the existing threshold. Returned `Surface.color.a` is ignored. | Returned alpha sets blend opacity, as water and glass shaders need; no `discard`. Translucent draws are in neither optimization, so this costs nothing. | Eligible for vanilla-coverage visibility IDs and a truly discard-free opaque fragment. |
| `pack` | Returned alpha controls the existing alpha test; pack code may discard. No fragment-depth writes. | Returned alpha controls opacity and discard. | Use forward shading and the general opaque/cutout pipeline for affected terrain. Retain ordinary culling, compact storage and OIT when supported. |

The checker walks reachable Naga functions and nested blocks. Refuse `discard` reachable from `material` under
`engine`, including shared helpers; allow it under `pack`. This is needed because `check.rs::api_rules` currently
checks names/bindings/entry points but does not forbid a pack's own discard. Retain `Surface`'s one-field shape.
The opaque entry and everything reachable from it must contain no discard, including displacement selection.
Do not execute a cutoff with a constant-true condition and call that a discard-free pipeline.

Engine coverage does **not** promise unchanged geometry. `[vertex]` and `[displacement]` move geometry independently.
The ID/depth/shadow passes must use the same moved vertices. Until displaced triangles have reconstructible IDs,
they use forward rendering even under engine coverage. Keep replacement-base-face discards out of the opaque
variant by draw-list partitioning, or retain the general variant for those draws. Animated geometry also needs
previous positions for correct motion and tracing; the coverage declaration cannot provide them.
Changing holes means `coverage = "pack"`; normal/parallax shading that keeps the silhouette does not.

Apply the same engine-default rule to `[entity] coverage` now, without terrain's translucent exception: an entity's layer opacity stays the engine's under `engine`. Engine entity cutoffs, dissolve masks, culling and
layer opacity remain engine-owned; `pack` opts into returned alpha/discard. This prevents inconsistent coverage
contracts when entity surface guides, ray hits and posed pipelines arrive. It does not make entities eligible for
the terrain visibility buffer. Extend shadow/ray coverage only when those paths can evaluate the same rule;
otherwise report the unsupported combination rather than claiming the visible and traced surfaces agree.

## Clouds and other engine-drawn light

**In now:** specify one `[environment]` stage instead of the unshipped `[glow]` name.
`fn environment(input: EnvironmentInput) -> vec3<f32>` shades engine-drawn sun, moon, stars, clouds, lightning,
beacon/End-crystal beams and additive particles. A cloud is not a glow, so the broader name matters.
Use `kind: u32`: 0 sun, 1 moon, 2 stars, 3 clouds, 4 lightning, 5 beacon beam, 6 End-crystal beam, 7 additive particle.
Engine-supplied inputs may gain fields. Supply linear `color`, `screen`, `direction`, `position`, `normal`,
`light_source` and validity bits for position/normal (celestials are directions, not nearby surfaces).
`color` includes the engine texture/tint/shade, before fog and blending. A missing stage returns that colour.
`light_source` uses the existing artistic source scale, not nits or physical irradiance.
Engine retains coverage, depth, draw order and each draw's default blend operation.
Clouds remain alpha-blended/depth-tested; sharing a callback does not make them additive. Route OIT variants too.
Ordinary lit particles stay `[entity] kind = 4`; additive particles use `environment`, never both.
Each new `kind` must have a defined fallback and a test of its caller; unknown kinds may use `input.color`.

The sun needs one extra control. Today's look changes the destination gain and clips the sky-plus-sun result
in two passes (`fs_sky_sun`/`fs_sky_sun_clip`); a colour-only return cannot reproduce it. Add the stage-local
helper `set_environment_composite(destination_gain, ceiling)`: gain is a nonnegative linear-light multiplier
on the default destination contribution; ceiling is a linear RGB bound on the composed result. Default gain 1
and no bound preserve the original blend. The executor converts these to the stored frame's basis and performs
the neutral affine/min operations. The pack supplies all weights and peaks, including the soft halo transition.
Internal blend alpha used for this operation is not surface coverage. No Horizon sun constants stay in the engine.

**Additive later:** a pack's own volumetric clouds use `[sky] clouds = false`, then post or compute before resolve.
They need R depth, the jittered inverse projection, sun direction (already present), time/weather, and kept
colour/transmittance history. World-space caches use the pre-world compute slot; depth-based integration uses the
scene chain. Denoise there before engine resolve; do not accumulate the tone-mapped sky. Horizon supplies no
mandatory cloud density/noise model. Separate sun colour/irradiance and horizon-sky inputs can be added when an
engine source exists; `Frame.sky_color` is one vanilla colour, not a physical sky model.

## Mods: one route for lit models

| Geometry today | Receives pack lighting today? | Decision |
| --- | --- | --- |
| Content-mod blocks/items in ordinary engine streams | Terrain yes; items have the same remaining #14583 caller gaps as vanilla items. | **In now:** preserve the normal route; do not special-case vanilla namespaces. |
| Native terrain-sheet models, `fs_native_model` | No. `gfx.rs` selects `terrain_model_pipelines`; its shader bakes light at vertices and bypasses `[entity]`. | **Additive later:** route through `[entity] kind = 1`, carrying tint, shade and light apart. Include atlas/LabPBR access and its own cutoffs. Required before claiming mod-model lighting parity. |
| Native still-fluid models | Separate `fs_native_fluid`, not the terrain material stage. | **Additive later:** a material-aware adapter preserving their depth/blend policy; do not imply the model fix covers them. |
| World primitives, canvas text, debug lines | Intentionally unlit/precoloured; some canvas runs are explicitly marked `NONE`. | **No:** automatic relighting. Keep their existing appearance and frame conversion. Lit machines use the model route. |
| A mod's raw wgpu render hooks | No automatic shader-pack stage evaluation. `WorldDrawn` currently follows native tone mapping. | **In now:** define the frame contract; **additive later:** a scene-draw hook for raw geometry before post/resolve. |

The mod-author rule is: submit lit objects through Horizon's model path to receive the active pack's lighting.
Raw draws own their shading and must follow `hdr-passes.md`. Give raw-hook events read-only encoding, primaries,
scene/display phase, paper-white units, R/O sizes, jittered/unjittered matrices and history-reset information.
An RGBA16F format alone does not say gamma or linear, or whether the curve already ran. Never guess from pack
presence. Keep the existing late `WorldDrawn` at a documented phase; a new earlier hook is a separate capability.
That hook and mod API changes belong with arismoko/horizon under CONTRIBUTING, not an upstream mod-API PR.
`ExternalPipeline` remains internal and is not the mod interface. World-warp pipelines currently replace pack
material shading; composable warped lighting/motion is a separate follow-up, not fixed by this note.

Native coloured visual lights already reach `colored_light`. Keep them separate from vanilla light levels;
convert their gamma RGB to working-space linear once. The current engine adds them per vertex and clamps there;
native material samples them per fragment. **In now, #16126 step 10:** add `shading_with_vertex_lights` for the
interpolated engine result, preserving `shading` and `colored_light` meanings. A physical pack may use the latter.
Native `emissive/` texture pairs and resource `_e` overlays must reach the same sampled emissive-overlay input.
They are visual emission tags, not a request to change gameplay block light or automatically add an RT light.

## One material vocabulary, not three lighting models

**In now:** use these meanings for raster terrain, entities and #16011 ray-hit additions. Keep pack lighting in
shared WGSL. No engine BRDF or mandatory energy scale is introduced. Existing fields retain their meanings.

| Quantity | Meaning and source |
| --- | --- |
| Base colour | Unlit texture and tint in working-space linear light. Keep vanilla `shading`, directional shade and `lightmap` separate. A ray hit must not secretly return already-lit colour. |
| Geometric normal / tangent | World axes; tangent xyz points along increasing u, w makes `cross(normal,tangent.xyz)*w` point along increasing v. Shading normal from LabPBR is separate from geometric visibility/offset normal. |
| LabPBR normal/height | Raw data: normal xy, AO, height; no colour conversion. Height moves rays/sampling unless `[displacement]` explicitly moves geometry. |
| LabPBR specular | Raw perceptual smoothness, F0/metal code, porosity/subsurface, emissiveness. Do not rename smoothness to roughness without conversion, or treat metal codes as F0. |
| `MaterialInput.emission` / `emission_threshold` | Legacy block source hint and the bright-texel mask threshold. Neither is LabPBR emission or emitted radiance. |
| `EntityInput.emission` / `light_source` | Full-bright layer flag versus artistic source strength. Full-bright eyes are not a physical intensity measurement. |
| Emissive overlay | Linear sampled RGB plus its own mask, composited once inside the pack's material model. It does not change the surface's engine coverage. |

Material helpers already sample block `_n`/`_s` maps. `EntityInput` has no maps or tangent; #16011 item 1 adds
hit-map access and tangent. **Additive later:** entity maps and a common opt-in sampled-material view for all
three paths, with per-map validity and neutral fallbacks. Preserve `Surface` and extend inputs/helpers instead
of adding required constructor arguments. New helper names require a manifest opt-in or a new stage.
Defaults are flat normal, AO 1, no relief, dielectric/no authored metal and zero emission. LabPBR specular alpha
1 is its non-emissive sentinel; missing maps return zeros today. Decode both correctly; never use alpha as gain
without its LabPBR rule. A shared SDK decoder can serve all stages when the bindings exist.
An authored emission map or overlay is an explicit signal; the SDK applies it before considering legacy source
hints as a fallback. A pack may choose another look. The engine never adds its HDR lift over a pack's answer.

**In now, #16126 step 10:** expose the sampled `_e`/native emissive overlay to material instead of a late
engine-owned blend that the pack cannot light. The built-in terrain branch has that sample; `NativeMaterial`
currently omits it. Entity overlays may be separate full-bright layers; tag them and avoid a second composite.
Keep glint and hurt-flash policy explicit; glint is currently added after `[entity]`, and changing that boundary
needs #16126's composed-light inputs and captures. Do not equate an `_e` mask with `emission_threshold`.
**In now:** add `[entity] fog` with the material stage's meaning and default `true`. `NativeEntity` currently
always applies fog after the stage; a traced pack that fogs terrain itself needs to opt out for entities too.
Fix the tangent sign bug tracked by #16082 against its reference, then compare raster/hit UV rotations before
using normal maps as denoiser guides. This is a correction, not another planned API break.

## Ray tracing, motion and kept resources

**Additive later, #16011 item 4:** declare typed targets and buffers, with a pass list that interleaves post
functions and compute dispatches in source order. Keep the ordinary colour-chain size R. Support fixed sizes
such as 1×1 and 64×64 as well as R-relative sizes; `rgba16float` colour and unfiltered `r32float`/`rgba32float`
data targets; bounded read-write storage buffers with a declared byte size. Data channels, including alpha,
retain their values and are not colour-decoded, clamped to headroom or forced to 1.
Keep WGSL bindings and compute entry points engine-generated. Gate compute, storage and format use on actual
device features/limits, with a reported unsupported requirement; never silently substitute half precision.
Require declared reads/writes and one writer per resource per pass; reject attachment/sample aliasing.
Use ping-pong for a kept texture's previous/current versions. A read-write cache buffer may update in place
within a compute dispatch; race avoidance is the pack's algorithm. Later draws only read it, with explicit
pass boundaries. Dispatch bounds, workgroup size, aggregate memory and pass counts are checked/budgeted.

Offer `before_world` compute after current ray-scene preparation for a cache read by `material` in that frame.
Offer ordered scene-chain compute after rasterization for exposure reduction and temporal denoising.
A compute pass immediately before post satisfies Evan's Q6 cache request; post-only caches need not run earlier.
Writes after a material draw can only affect the next draw/frame, never retroactively change that material.
Trace remains an engine helper, including in compute when requested; packs see no acceleration structures.

Kept storage is per view, not shared globally. Screen history resets on the events above. A declared world-space
cache survives resize/display changes; clear it on world/dimension change, resource or pack reload, a relevant
option change, device loss, or teleport beyond its declared reach. Reset ages/keys as well as radiance.
Expose reset reasons and a monotonically changing world/history epoch; pack code must not infer a world change
from camera distance or `frame_index` alone. No persistence across sessions.
Budget for the concrete Q6 request: two buffers of 2^18–2^20 cells × 16 bytes, **8–32 MiB total**, per view.
Also support the small kept exposure target and a 32-bit distance history. These are new typed resources;
today's single half-float `history` and named-target alpha cannot supply them.

`trace` in post fits the scene chain, before resolve. Tracing opaque lighting once per visible pixel is cheap
relative to tracing each material fragment. Water/glass still need `trace` in material before blending.
Translucent ray layers and entity BLAS are #16011 items 2/3; phase changes do not add them, displaced geometry,
far terrain or the hand to the ray scene. A temporal denoiser runs in compute/post at R before engine resolve.
It can use jitter and matrices now. Entity histories require object motion or explicit rejection.

**Additive later:** engine-owned guide resources, requested by a pack or the #12926 backend. Use one shared
set for temporal packs, upscaling, generation and ray denoisers. Do not make each pack rerender entity motion.

| Guide | Contract |
| --- | --- |
| Motion | R-sized float texture; xy = `previous_uv - current_uv`, normalized top-left UV, from **unjittered** projections. Separate validity/coverage identifies camera-only estimates, matched objects and invalid/new geometry. Signed vectors are data. |
| Depth | R-sized reversed depth, 1 near and 0 sky, with the matching jittered projection/inverse and near/far metadata. Convert for a backend only at its adapter. |
| Albedo / normal | Unlit working-space base colour and world-space shading normal, independently valid per pixel. Preserve geometric normal separately where ray offsets need it. |
| Reactive/transparency information | Explicit mask for blended/animated emissive surfaces; a camera vector is not an adequate substitute. |

The current #12926 seam is camera-only. Frame generation already has a separate object-motion pass matching
stable entity vertex topology; reuse that work, but its unmatched objects and pack-deformed vertices are not
complete motion. `surface_at` is terrain-only normal/smoothness/custom data, not an albedo/entity G-buffer.
Expose capabilities and invalid regions honestly. A DLSS/FSR-style backend that requires full object motion
cannot be declared supported just because camera reprojection exists. No vendor SDK enters the shader API;
FSR may be an in-repo backend, DLSS remains an optional native mod under CONTRIBUTING.

## Existing packs and other small contract decisions

| Item | Decision |
| --- | --- |
| Golden Hour | Both post passes now run at R. Its UV grade and height-scaled glow need no new algorithm; migrate any pixel offsets to `target_size`/`input_size`. It defaults to engine AA. Recheck 50/100/150% scale and TAA, not just 100% with AA off. |
| `horizon_hdr` | No post chain to resize. Keep engine AA/coverage defaults. Its tone curve now receives resolved O light and runs on every output; sampled-light rounding can change. Its alpha return stops controlling terrain coverage. Add environment/overlay callers as they land. |
| Other 0.2 users | Last-post window pixels and arbitrary material/entity alpha cease to be defaults. Own AA needs the declaration. Own holes, and own opacity on solid terrain or entities, need `coverage = "pack"`; translucent terrain's opacity stays the pack's under `engine`. Recheck depth reconstruction when jitter starts. Update manifests, templates, checker, generated reference and examples atomically with each behaviour change. |
| Names and structs | **In now:** retain #14584's gated-name rule, nonconstructible engine inputs and fixed `Surface`. Do not rename existing `emission`, `resolution` or `normal` for tidiness. Explicit sizes, units and validity remove the ambiguity. |
| Colour metadata | **In now:** engine resolve/generation inputs carry format, transfer/storage encoding, working primaries, paper-white scale and reset state. The all-linear post result is not the original gamma frame. Keep float light, tone-weight linear temporal blending, and adapt to library colour contracts once. |
| Physical units / wide colour | **No:** change 1.0 from paper white to nits or install an engine light model. **Additive later:** serve #14584 BT.2020 once all outputs/HUD paths work. No HDR10-to-BT.2020 conversion twice. |
| Hidden outputs | **No:** implicit fragment depth, raw acceleration structures, arbitrary bind groups, output transfer encoding by packs, or unbounded allocation. These defeat engine ownership, portability or the coverage contract. |

## Pull requests, in dependency order

Sizes are estimated **added plus removed source lines per PR**, leaving room below 500; not measured diffs.
Documentation and separate fixtures are additional. Re-split a row if actual churn exceeds its range.
This is the phase/coverage design's slice of #16126, not a new estimate of its entire default-look migration.

| Row | Reviewable change | Size / dependency / place in #16126's plan |
| --- | --- | --- |
| 1 | Land this contract and reconcile #14035's old “replace world_post” wording; keep API experimental. | Docs only; before implementation (this note). |
| 2 | Finish #14680's native `ExternalPipeline` adapter without changing pictures. Split neutral layer, scene and mapping hooks across PRs. | 3–5 × 250–450; #16126 step 5, after its M6 prerequisites. A pure file move may be separate under M6's exception. |
| 3 | Split native scene-chain execution from pure output mapping, preserving existing behaviour first. | 250–400; after 2. Entry seam for #16011 item 4. |
| 4 | Switch final post to R; add pass/O sizes and AA ownership; route spatial resolve/FXAA/CAS; migrate both packs and reference. | 300–450; after 3; breaking change 1, replaces #16126 step 6's opt-in. |
| 5 | Make one jitter/camera snapshot; add actual/unjittered/previous matrices and history validity; enable pack temporal jitter. | 300–450; after 4; first part of #16126 step 7. |
| 6 | Route TAA/#12926 over the pack result with encoding metadata, tone-weighted linear history and joint resets. | 300–450; after 5; remainder of #16126 step 7. No vendor backend or complete motion claim. |
| 7 | Enforce engine-default terrain/entity coverage; reachable-discard checking; wrapper alpha rules; migrate examples. | 250–450; independent of rows 2–6: a checker rule and the wrapper's alpha, which the move carries unchanged (the `[entity]` half lands with the entity stage, #15942); breaking change 2. |
| 8 | Native discard-free opaque/cutout variants, including draw-list handling of displaced replacements. | 200–350; after 7 and compact terrain (#16126 step 2). First part of #16126 step 14. |
| 9 | Sampled terrain emissive-overlay input and single composition; interpolated vertex-light shading input, separately. Include the small entity fog opt-out in the entity-wrapper work. | 2 × 200–400; after row 2; within #16126 step 10, beside its glint/back-face work. Before row 10, which needs these inputs. |
| 10 | Analytic gradients; native visibility material resolve; surface output; IDs/pages; backend/fallback integration, each separately. | 5 × 200–450; after rows 8 and 9. Remaining visibility work in #16126 step 14. No displaced-triangle IDs in this slice. |
| 11 | Environment API/check/reference; celestial callers; cloud/OIT callers; lightning/beam/additive callers, each separately. | 4 × 200–450; after row 2; #16126 step 9, replacing the proposed glow name. |
| 12 | Raw render-hook encoding/phase descriptors in the mod fork, with explicit late-hook semantics. | 150–300; after phase contract; coordinate with 2. Not a public `ExternalPipeline` API. |
| 13 | Resource-post retained depth and order; per-output tone mapping/capture, separately. | 2 × 250–450; after 6; #16126 step 12. Do not fold the hand-layer redesign into these PRs. |
| 14 | Feed resolved light to generation; map real/made outputs identically; encoding/history integration, separately. | 3 × 200–400; after 6/13; #16126 step 13. Hold off jittered/deformed geometry combinations whose motion is invalid. |

Only docs/API design, #16126's existing steps 1–4 and the coverage rule (row 7) proceed before the executor adapter. Runtime rows above
wait for #14680 so the same integration is not rewritten twice. API declarations must not advertise runtime
support before their caller lands. The environment API can be reviewed earlier, with explicit runtime refusal.
The coverage default enables optimizations; it does not implement compact input expansion or visibility resolve.
Complete the rest of #16126 (posing/items/OIT, composed light, fog/fade, target reuse, capture replay and bundling)
before its step 16 flips the default. #16011 items 1/2 can continue independently; item 3 follows entity rendering.
Typed kept resources, compute, entity maps, guides and pack-owned temporal upscaling get separate designs/PR
budgets under #16011/#12918. Do not make their implementation a prerequisite for a static bundled look.

Every behavioural PR needs focused checker/GPU evidence. Retain no-pack SDR identity and the stated look-gate
limits. Check both storage encodings, 50/100/150% scale, above-white light, cutouts and transparency.
Use moving sequences for jitter, entities, cuts, reloads and generation; still frames cannot accept temporal work.
Metal must show that its actual opaque pipeline contains no reachable discard and that HSR benefits it.
Visibility tests must establish that the visibility route ran, including a pack-coverage fallback case.

## Questions for maintainers

| Question | Recommended answer |
| --- | --- |
| Broaden `[glow]` before it ships? | Yes: `[environment]`, with kinds, engine coverage/default blends and a neutral affine/cap helper for the sun. Clouds need no emission-shaped API. |
| Put the hand into this phase change? | No. Keep its limitation explicit; require a separate colour/depth-layer design under #14583 before moving it ahead of tone mapping. |
| Is applying the coverage default to `[entity]` within the approved break? | Yes. Settle both now; otherwise the same optimization/guide problem returns as entities gain inputs. |
| Must complete motion and kept compute resources block #16126? | No. Freeze their phase/units/ownership contract now, implement under #16011/#12918. Reject unsupported temporal combinations visibly. |
