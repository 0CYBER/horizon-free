# Design: section geometry

Provisional answer to [arismoko/horizon#864](https://github.com/arismoko/horizon/issues/864):
geometry added by a mod belongs to one section compilation, replaces its previous
contribution on remesh, and dies with that section's lifetime. Ari's side must
post the decision on the issue before implementation. No Rust API is added here.

[Section meshing](section-meshing.md) already proposes the worker capability,
Wire geometry, budgets, epochs and teardown needed here. Extend that contract;
do not create another context, store, callback bus or publication boundary.
arismoko/horizon#1508's returned comments contain only a claim, so approval is **not verified**.
This note adds the section-level append case rather than repeating its recipes.

## Vanilla and NeoForge 26.3

Verified from `/workspace/horizon` with `python3 tools/rea_vanilla.py method
net.minecraft.client.renderer.chunk.SectionCompiler compile` and the matching
`callers` command: `SectionRenderDispatcher.RenderSection.CompileTask.doTask`
is the sole reported caller (complete coverage). Compilation traverses 4096
positions, gathers block entities separately, emits fluid and model quads into
material layers, then sorts translucent output and resolves visibility. There
is no vanilla mod contribution event. These observations do not establish a
NeoForge event insertion point.

The issue names `AddSectionGeometryEvent`, distinct from per-frame
`SubmitCustomGeometryEvent`. The public [26.3 javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/client/event/AddSectionGeometryEvent.html)
was inaccessible during this audit. Exact event methods, timing, threading and
cancellation semantics remain unverified. Public [model documentation](https://docs.neoforged.net/docs/resources/client/models/modelsystem/)
was reachable but labelled 26.1; it does not verify this 26.3 event. Do not claim
full counterpart parity or freeze the public signature until that gap closes.
No NeoForge source/patches or other project's source were consulted.

## Proposed contract

Use `add_section_geometry` as the section append capability of the existing
frozen-plan preparation contract. Owner-thread preparation gets `ClientHost`,
resolves subscriptions and captures immutable inputs in decide/fire/apply order;
apply refuses changed preconditions. Ordinary mod code runs only there. A declared
worker-safe registration may instead emit one bounded batch per section using
the existing restricted worker Host. No Game, live world, mutable attachments,
scheduling, drawing or GPU access is available on workers. Dependency ordering
is identical for native and wasm; native captures are immutable and thread safe,
wasm uses isolated worker instances and bounded fuel. Both use the same API-owned
Wire values and resource keys. No borrowed engine structures cross the boundary.

Bind the batch before the block loop and merge its validated concrete geometry
into ordinary section layers, including empty-air sections (the current early
return must not discard additions). No per-cell callback or listener lookup.
Append never replaces block recipes, changes visibility or suppresses fluids.
Per-cell additions and model replacement remain arismoko/horizon#1508 recipes. The same quad
must be submitted through exactly one route. Translucent additions enter terrain
sorting; materials, prepared-model identities and colour follow section-meshing.

[arismoko/horizon#2333](https://github.com/arismoko/horizon/pull/2333)'s arismoko/horizon#772 proposal freezes
section and cell-face visibility from the same raw snapshot before dispatch.
Consume that decision once; do not ask visibility again inside this callback.
Recommend section hiding suppress all terrain contributions, while cell-face
masks affect only additions explicitly attached to that cell/face. Unattached
decorative quads remain unculled. Block-entity drawing retains its separate R2
lifecycle decision. Existing state visibility is reused, not redispatched.

Each job owns a lease on its frozen plan, callback instance and resource epoch.
Keep section lifetime/revision, halo revisions, resource/tint/mod generations in
the same result token used by arismoko/horizon#1508. Check again immediately before upload.
A resource-A job may finish after B but publishes zero A output. Coordinate reuse
gets a new lifetime. Cancellation is best effort; unloading removes the mesh and
prevents every outstanding result publishing. Remesh replaces the complete old
mesh, never appends to it. Teardown stops admission, increments generation,
cancels and drains jobs/results, then releases native code/wasm stores/resources.

Treat input and output as untrusted. Inherit arismoko/horizon#1508's 262144 quads/section,
32 MiB encoded section data and 128 MiB in-flight limits, checked before allocation;
cell-attached output also obeys 256 quads/cell and block-local [-1,2]. Recommend
unattached positions stay within section-local [0,16] on each axis so section
bounds remain sound. Validate decoded sizes, indices, tags, generation keys and
all finite vertex attributes; inherit UV magnitude <=65536, normal components
[-1,1], alpha [0,1]. NaN/infinity anywhere refuses the entire offending batch.
No partial append survives. Native unwind panics and wasm traps/fuel exhaustion
refuse that registration's batch, disable it for the generation with one diagnostic,
and invalidate affected sections; built-in geometry remains. Native aborts cannot
be recovered, and native execution cannot be safely forcibly preempted.

With no subscribers select the existing mesher at dispatch: zero extension
allocations, halo copies, callback calls or worker hot-path dispatch. Resolve the
subscriber presence flag once, O(1); do not scan mods. Performance is proposed,
not measured here; inherit the under-1% benchmark-noise target from arismoko/horizon#1508.

## Verified route owners

Paths relative to `crates/`, audited at `ee5816229c`. These are existing
integration points, not claims that proposed guards already exist. Shared
invalidation and publication rules above apply to every row.

| Route | Current file:line owner | Required extension |
| --- | --- | --- |
| Initial mesh and remesh admission | `horizon-client-render/src/streaming.rs:2237` | Freeze one shared plan before dispatch |
| Worker execution | `horizon-client-render/src/streaming.rs:1142`; `horizon-client-render/src/mesher.rs:1751` | Batch once; include empty sections |
| Block writes and border edits | `horizon-client-render/src/world.rs:385` | Invalidate shared halo dependencies |
| Remote chunk arrival | `horizon-client-render/src/streaming.rs:1302` | Capture replacement section and neighbour revisions |
| Biome/light replacement | `horizon-client-render/src/streaming.rs:3455`, `:1342` | Rebuild dependent plans |
| Resource and visibility refresh | `horizon-client-render/src/streaming.rs:1277`; `horizon-client/src/game/modding/native_render_visibility.rs:13` | Replace context and invalidate through one route |
| Unload | `horizon-client-render/src/streaming.rs:516` | New lifetime on reload; remove mesh, cancel old jobs |
| Abandoned/stale completion | `horizon-client-render/src/streaming.rs:1887` | Extend current sequence/column refusal with shared epoch token through upload |

## Acceptance matrix

Future implementation expectations; no runtime tests were run for this note.

| Case | Concrete expected value |
| --- | --- |
| Empty section plus one decorative quad | 1 quad after build and after each of 3 remeshes |
| Isolated cube plus one quad | 7 quads; callback executes once per compilation |
| Unload then late completion | 0 uploads and 0 retained section draws |
| A finishes after resource B | 0 A uploads; B's one-quad contribution remains 1 |
| Same coordinates, new lifetime | Old result uploads 0 times |
| Section hidden by shared visibility | 0 terrain quads, 0 visibility redispatches |
| NaN, infinity, 262145 quads, invalid material, native panic or wasm trap | 0 offending added quads; built-in cube retains 6 |
| Native/wasm same bounded input | Same vertex values, materials and quad count |
| No listeners | Baseline mesh identical; 0 extension allocations/callbacks |

## Proposed stages (file none)

1. Ari decides the shared append/visibility contract on arismoko/horizon#864 and coordinates arismoko/horizon#1508/#772; verify the exact public 26.3 counterpart before naming the API final.
2. After arismoko/horizon#1508's R2 prerequisites, implement shared job leases, budgets and publication guards for both targets together.
3. Add section batch adapters and a test mod; prove the matrix, unload/reload races and no-listener counters before performance measurement.

## Open questions

- Does arismoko/horizon#1508 subsume arismoko/horizon#864? Recommend yes for lifetime and execution; retain only this section-level append capability and its test-mod acceptance case.
- Should hidden sections hide additions? Recommend yes for terrain; decide once with arismoko/horizon#772, preserving separate block-entity lifecycle policy.
- May unattached geometry cross section bounds? Recommend no initially: [0,16] keeps culling and unload ownership explicit. Cell-attached geometry retains arismoko/horizon#1508's halo contract.
- Exact NeoForge counterpart contract remains unavailable. Recommend obtain public 26.3 javadoc evidence before implementation approval; do not infer semantics from older docs.
