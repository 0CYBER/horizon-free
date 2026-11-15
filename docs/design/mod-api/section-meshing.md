# Design: section meshing

Proposal for [arismoko/horizon#1508](https://github.com/arismoko/horizon/issues/1508), requiring
maintainer approval before code. Implementation follows R2 GPU instancing
([arismoko/horizon#808](https://github.com/arismoko/horizon/issues/808)); this page does not claim
that prerequisite is complete. No Rust API is implemented here.

## Scope and reference

Extend terrain geometry preparation, not the renderer. Full mesher replacement
is out of scope: replacing scheduling, visibility, buffers and upload ownership
would defeat the frozen-plan and shader contracts. Renderer ports can use this
contribution seam but cannot install an arbitrary section compiler through it.
Fluids retain their existing mesher; replacing block geometry does not remove
fluid geometry. Block entities drawing themselves use the R2 lifecycle decision,
not a second geometry hook at the same moment.

Vanilla 26.3 `SectionCompiler.compile` traverses the section, collects block
entities separately, emits fluid and model geometry into material layers, and
sorts translucent quads. Verified with `python3 tools/rea_vanilla.py method
net.minecraft.client.renderer.chunk.SectionCompiler compile` from
`/workspace/horizon`; its callers are queried with the corresponding `callers`
command. The proposed frozen plan is Horizon policy, not a vanilla hook.
The issue identifies only a partial NeoForge counterpart: model data and model
quad hooks, not whole-section replacement. Public
[model documentation](https://docs.neoforged.net/docs/resources/client/models/modelsystem/)
is contextual reference, not a claim of an identical event contract. No external
project implementation is a design input.

## Owning thread and frozen decisions

Register subscriptions by block or exact state during client registration;
resolve keys to dense state slots once per resource epoch. Exact-state rules
follow block rules, listeners in dependency order. One client-side preparation
moment receives a `ClientHost` and immutable section input on the world-owning
thread. Client-lib decides, game fires, client-lib validates and applies in one
transaction; if section or resource preconditions changed, refuse and reschedule,
never silently recompute inside apply. No new broad Host service is needed.

Freeze a section plan with shared state recipes and sparse position overrides
for neighbour-dependent choices. Resolve model selection and culling there.
Workers consume immutable arrays and prepared geometry references, never a live
world, game handle or ordinary mod callback. A state recipe selects a prepared
model, appends quads, or replaces its block-model quads (including with zero).
Culling is a six-face mask plus explicit full-face occlusion declarations;
unculled decorative quads are permitted. Replacement clears the original geometry
before append. Multiple appenders compose; reject competing replacements for the
same state at registration with both owners named. Do not let load order silently
choose a replacement.

Snapshot the 16-cubed section plus a one-block halo on all six sides, including
edges and corners: 18 cubed = 5832 state cells, with immutable light, biome and
availability inputs. Model selection may inspect only that radius. Missing
neighbours are explicitly `Missing`, never a fabricated state id; for selection
and culling treat them as nonmatching, nonoccluding air. Outside world height is
known air. Arrival or disappearance invalidates every dependent border section.
Larger-radius connected textures are out of scope. Freeze deterministic position
seeds; worker order cannot change model choice. Biome blending may use the
engine's separately captured blend footprint; it does not enlarge mod geometry
neighbour access.

## API-owned data and worker execution

Proposed API-owned Wire representations are `SectionMeshPlanWire`,
`SectionMeshInputWire`, `StateMeshRecipeWire`, `MeshQuadWire`, `MeshVertexWire`,
`MeshFaceMaskWire`, `PreparedModelKeyWire`, `MeshMaterialKeyWire`, and
`WorkerMeshRegistrationWire`. These are bounded values with explicit integer
widths, enum tags and lengths; no game structs, GPU layouts, pointers or borrowed
references cross native/wasm boundaries. Model and material keys carry resource
generation and resolved slots; unknown or stale keys refuse that contribution.
Wire here is the mod boundary, not a multiplayer packet.

A declared worker-safe registration is a separate execution capability within
this preparation contract, not another all-mod event bus. Give it a restricted
read-only worker Host over captured input and registries, with no world writes,
attachments mutation, scheduling, drawing or live Game access. Native captures
must be immutable and thread safe; wasm uses isolated worker instances with the
same imports, budgets and dependency ordering. Bind callbacks once into batches
of subscribed states/positions. Geometry callbacks emit bounded section batches
before the ordinary block loop; that loop reads concrete data only, with no
per-block trait objects, wasm calls or dynamic listener lookup.

[arismoko/horizon#2305](https://github.com/arismoko/horizon/pull/2305)'s biome colour resolvers
already require worker colour execution. Preserve that declared worker-safe
capability: capture resolver state and lifetime with the job, use the existing
tint evaluation/cache path, and forbid live mutable world access. Its colour
callbacks are not implicitly geometry callbacks. The no-per-block-dispatch
requirement applies to this new geometry seam; it does not assert existing tint
callbacks never execute. Owner-prepared tint values remain valid where supported.
Teardown first stops scheduling and increments mod generation, then cancels jobs,
drains callback instances and results, releases plans, and only then unloads
native code or wasm stores. A job lease prevents calling unloaded code.

## Rendering and colour ownership

Use R2b prepared model geometry as the canonical baked source, sharing immutable
geometry and material identities instead of exposing atlas offsets or GPU device
objects. Section emission expands that geometry through the terrain path;
instanced self-drawing block entities remain on R2. Freeze the self-draw skip
bit before terrain preparation: it suppresses that entity's ordinary block-model
geometry once, without suppressing unrelated extra geometry or fluids. Never
infer self-drawing from merely having a block entity.

Opaque and cutout output retain terrain depth/culling policy. Translucent output
enters the existing translucent sorting and upload path, never the opaque buffer
because a mod requested an arbitrary shader. Cull only declared full occluding
faces; translucent neighbours do not acquire opaque occlusion automatically.
Shader packs consume the same material attributes as vanilla terrain; a mod
cannot choose a raw pipeline, blend mode or frame format here.

Follow [arismoko/horizon#1498](https://github.com/arismoko/horizon/issues/1498) and
[the colour design](../hdr-linear-light.md): shader API 0.2 colour
is linear light with BT.709 primaries, paper white 1.0, and values above 1.0
retained. Existing packed ARGB tint/light inputs remain vanilla-space inputs;
renderer conversion happens once. Encoded tint 0.5 becomes approximately
0.21763764 under power 2.2; linear shader output 2.0 stays 2.0 until output
mapping. UV, normals, depth and masks remain data. Alpha and compositing belong
to the renderer. Pack changes rebuild plans only when geometry/material/colour
interpretation changes; render-only changes rebuild pipelines without needlessly
re-running geometry callbacks.

## Route ownership and invalidation

Verified owners at worktree base `5c2583c030`; paths below are relative to
`crates/`. These are current integration points, not claims that proposed epoch
checks or callbacks already exist.

| Route | Current file:line owner | Required design behavior |
| --- | --- | --- |
| Initial dispatch and worker mesh | `horizon-client-render/src/streaming.rs:2237`, `:1142`; `horizon-client-render/src/mesher.rs:1741`, `:1751` | Capture plan before dispatch; mesh immutable snapshot |
| Block/state write, neighbours, borders | `horizon-client-render/src/world.rs:385` | Dirty all sections intersecting radius-one dependency, including vertical and diagonal borders |
| Chunk arrival, biome and light updates | `horizon-client-render/src/streaming.rs:3455`, `:3466`, `:3476`; `:1302`, `:1341` | Invalidate dependent halo plans even if centre states did not change |
| Unload | `horizon-client-render/src/streaming.rs:516`, `:1394`, `:1961` | Invalidate lifetime generation, cancel jobs, remove uploads; dirty surviving neighbours |
| Resource/model replacement | `horizon-client-render/src/streaming.rs:1277`, `:1283`; `horizon-client/src/game.rs:14161` | Atomically replace WorkerContext and prepared geometry epoch; rebuild loaded sections |
| Biome-tint refresh | `horizon-client/src/game/modding/native_biome_colors.rs:16` | Capture new colours/resolvers and remesh through resource reload |
| Shader/renderer rebuild and pending uploads | `horizon-client/src/game.rs:14161`, `:14178` | Clear pending old output; rebuild affected plans through shared invalidation |
| Cancelled/stale completion | `horizon-client-render/src/streaming.rs:1887` | Refuse results with mismatching token or absent column; recycle buffers |
| Prepared geometry generation | `horizon-client-render/src/gfx/prepared_model.rs:268`, `:273` | Keep old resources leased while jobs run; refuse old-generation publication |
| Block entities and layer emission | `horizon-client-render/src/mesher.rs:1751`; vanilla `SectionCompiler.compile` | Integrate proposed R2 skip decision here; retain separate self-draw lifecycle and translucent sorting |
| Native/wasm tint and teardown | `horizon-client-render/src/mesher/mod_tint.rs:20`, `:110` | Extend captured data/lifetime boundary; proposed geometry adapters belong in game, not render crate |

Every job/result carries section lifetime, section revision, halo revisions,
resource epoch, tint generation and mod generation. Publication checks all of
these again immediately before upload. Unload/reload at identical coordinates
creates a new lifetime. Cancellation is best effort; even a completed cancelled
job cannot publish. Old epoch A resources stay alive for A jobs but their results
cannot replace epoch B. Snapshot leases and result memory count against admission
budgets until dropped; never reuse an old plan against newly baked geometry.

## Bounds and performance

Proposed hard limits: 256 contributed quads per block position, 262144 contributed
quads and 32 MiB encoded contribution/plan data per section, and 128 MiB total
in-flight extension data. Count shared recipe storage once globally and expanded
output per job; checked arithmetic precedes allocation. Pause new extension jobs
at the global limit, rather than evicting live leases. Validate every vertex,
UV, normal and colour for finiteness; positions must be in block-local [-1,2],
UV magnitude at most 65536, normal components in [-1,1], and alpha in [0,1].
Geometry cannot extend beyond the halo. Reject invalid indices, material keys,
lengths and enum tags before baking. Enforce both encoded and decoded budgets.

Refuse a failed mod's entire section contribution atomically and use original
built-in geometry, including original culling, without partial replacement.
Catch unwind-capable native panics at the adapter; wasm traps/fuel exhaustion
have the same refusal. Disable the faulty registration for the generation with
one diagnostic, invalidate affected sections and continue built-in rendering.
Native aborts cannot be recovered; worker-safe native code must use the supported
unwind boundary. Wasm batches receive a finite fuel quota; native callbacks have
no safe forced preemption and remain subject to the declared worker-safe contract.

With no geometry subscriptions, use the existing mesher entry directly: no plan
allocation, halo copy, listener scan or per-block branch added by this feature.
Select the path once at dispatch. With subscriptions, share dense state recipes,
reuse scratch buffers and precompute sparse overrides. Acceptance requires
allocation/callback counters at zero for the empty extension path and measured
no-mod meshing throughput within benchmark noise (target under 1% regression);
this document makes no measured performance claim.

## Acceptance cases

These are future implementation tests, not results of this documentation change.

| Input | Expected value |
| --- | --- |
| Isolated opaque unit cube | 6 quads |
| Two adjacent full opaque cubes | 10 exterior quads |
| Replace isolated cube with empty recipe | 0 model quads, no retained original six |
| Append one unculled quad to isolated cube | 7 quads |
| Two cubes across section border; neighbour unloads | Centre contribution changes from 5 faces to 6; reappearance returns to 5 |
| Connected model on east/west matches | Neither: model 10; east only: 11; west only: 12; both: 13; Missing counts as false |
| Write at local (15,15,15) with surrounding sections loaded | Exactly 8 radius-one dependent sections dirtied |
| Finish A after resource B or unload/reload | 0 A uploads; B remains visible |
| Self-drawing block entity with six-face model | 0 ordinary model faces; one R2 draw; fluid retained |
| Two translucent quads at distances 2 and 5 | Both in translucent output, distance 5 ordered before 2 |
| NaN vertex, 257 quads at one cell, panic or wasm trap | Whole offending contribution refused; isolated built-in cube has 6 faces |
| Reload during worker colour callback | Old callback lease survives to completion; old result has 0 uploads |
| Native and wasm same bounded plan | Identical model ids, face counts and materials |
| Repeated empty-listener runs | Identical baseline geometry; 0 extension allocations/callbacks |

## Open questions

- Final R2b geometry identity and R2 self-draw extraction type are still moving.
  Recommend binding the proposed keys/skip bit to those approved types after
  arismoko/horizon#808, retaining the generation and lifetime rules above.
- Limits need representative mod scenes before implementation approval.
  Recommend the explicit bounds above as initial ceilings, then measure dense
  connected geometry and wasm batches before increasing them; do not ship an
  unbounded escape hatch.
