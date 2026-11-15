# Design: mesh visibility

Provisional answer to [arismoko/horizon#772](https://github.com/arismoko/horizon/issues/772):
freeze section-wide and per-cell face decisions before worker dispatch, expose
raw immutable section input, and invalidate by captured revisions and epochs.
Ari's side posts the decision on the issue; this note implements no Rust API.
It extends the [section meshing plan](section-meshing.md) for arismoko/horizon#1508 and shares
its input, budgets and publication boundary with arismoko/horizon#864, rather than introducing
another meshing context or worker event bus.

## Vanilla and NeoForge 26.3

Verified vanilla using `python3 tools/rea_vanilla.py method
net.minecraft.client.renderer.chunk.SectionCompiler compile` and `callers` for
that method from `/workspace/horizon`. Compile visits 4096 cells, marks solid
cells in VisGraph, collects block entities, emits fluids separately from block
models, sorts translucent output and resolves section connectivity. The sole
reported caller is `SectionRenderDispatcher.RenderSection.CompileTask.doTask`.
These are separate geometry and connectivity decisions; hiding geometry must
also remove its occlusion contribution to avoid invisible walls hiding terrain.

The issue explicitly identifies no NeoForge counterpart. Exact 26.3 absence
cannot be independently proved from the available public references. The public
[model documentation](https://docs.neoforged.net/docs/resources/client/models/modelsystem/)
is labelled 26.1 and describes directional culling and model data, not this
contract. The attempted [26.3 event Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/client/event/RenderSectionGeometryEvent.html)
was inaccessible; exact 26.3 behavior remains unverified. No NeoForge source,
patches or other project's source informed this proposal.

## Proposed contract

Keep the existing state visibility registration as the shared default recipe;
add section and face inputs at the same preparation moment, with one canonical
visibility plan. Final public naming awaits approval; no parallel legacy hook
or compatibility alias. Resolve subscriptions to dense state slots at load.
Client-lib captures input, game dispatches with ClientHost on the client
world-owning thread, then validates and applies in one transaction. Refuse
changed preconditions and reschedule; never recompute inside apply. Workers
consume concrete frozen arrays only, with no new mod dispatch, live Host or
world access. arismoko/horizon#1508's explicitly declared worker-safe geometry/tint capability
is separate and does not authorize visibility callbacks on workers.

A section decision is Keep or HideTerrain. HideTerrain emits zero block-model,
fluid and arismoko/horizon#864 terrain-contribution quads and contributes no occlusion. It does
not suppress independently drawn block entities, entities or collision. Keep
permits per-position decisions: block-model visibility, fluid visibility and a
six-bit directional face mask, ordered down, up, north, south, west, east.
Default mask is 63. Suppression composes by intersection in dependency order;
no listener can restore another listener's hidden geometry. Section suppression
wins. Unculled decorative quads follow the position's model visibility, not an
invented directional classification. arismoko/horizon#864 unassociated section contributions
follow section visibility; position-associated quads follow their position and
declared cull face. Replacement and append retain arismoko/horizon#1508's conflict rules.

Face bits filter eligible geometry after normal culling; they do not force
normally occluded faces to appear. A wholly hidden cell is nonoccluding, as in
the existing hidden-state behavior. A partially masked cell retains full-face
occlusion only on retained, explicitly full occluding faces. Recompute section
connectivity conservatively from effective occlusion; uncertainty must allow
visibility rather than incorrectly cull terrain. Translucent faces remain in
the translucent sorting path and never become opaque occluders. Fluid masks
filter emitted fluid directions without changing the captured fluid simulation.

Raw means API-owned state ids and immutable captured values before visibility
substitution, not palette internals, mutable chunks, pointers or GPU buffers.
Reuse arismoko/horizon#1508's 18 cubed snapshot: 5832 cells including edges and corners, with
light, biome, availability, section coordinates and deterministic seeds.
Reads are bounded to local [-1,16] on each axis; outside reads refuse. Missing
is distinct from air but acts nonoccluding/nonmatching for decisions; outside
world height is known air. Hidden ore remains present in raw input. No access
to unloaded terrain, block-entity mutable state or larger-radius live queries.

## Invalidation and verified owners

Inspected at base `8b736c3cbc`; paths relative to `crates/`. Existing owners
below are integration points, not claims that the proposed epochs already exist.

| Route | Verified file:line | Required behavior |
| --- | --- | --- |
| Initial mesh and remesh | `horizon-client-render/src/streaming.rs:2237`, `:1142` | Prepare before scheduling, use frozen input in worker |
| State write and neighbour borders | `horizon-client-render/src/world.rs:385` | Dirty all radius-one dependent sections, including diagonals and vertical borders |
| Column replacement, biome/light changes | `horizon-client-render/src/streaming.rs:3455`, `:3476` | Invalidate halo dependencies even if centre state is unchanged |
| Visibility toggle | `horizon-client/src/game/modding/native_render_visibility.rs:13` | Explicit refresh increments policy epoch and remeshes affected loaded sections; conservative all-loaded refresh initially |
| Resource reload | `horizon-client-render/src/streaming.rs:1277`, `:1283` | Replace context, rebuild plans and sections |
| Renderer recovery and pending uploads | `horizon-client/src/game.rs:14166`, `:14178` | Clear pending old uploads and use shared reload invalidation |
| Unload | `horizon-client-render/src/streaming.rs:516`, `:1394` | Cancel queued jobs, invalidate lifetime, remove terrain, dirty surviving halo dependents |
| Stale workers | `horizon-client-render/src/streaming.rs:1887` | Extend sequence/column checks to all captured generations and recycle refused output |
| Existing mask consumption | `horizon-client-render/src/mesher.rs:1766` | Preserve built-in behavior and raw input separately |

Chunk arrival also invalidates surviving sections that captured Missing; use
the streaming column installation boundary shared with replacement. Every plan,
job and pending upload carries section lifetime/revision, halo revisions,
resource, mod and visibility-policy epochs (and arismoko/horizon#1508 tint generation).
Check again immediately before upload. Unload/reload at identical coordinates
has a new lifetime; cancellation alone is insufficient. Old resources stay
leased until old jobs finish, but old results publish zero uploads. Arbitrary
mod state changes require explicit visibility refresh; no per-tick polling.

## Trust boundary, targets and cost

Native and wasm receive the same generated bounded Wire snapshot and return
identical integer masks; no borrowed game representation crosses either ABI.
Reject invalid tags, unknown/stale ids, duplicate coordinates, masks above 63,
and positions outside [0,15]. No floating-point visibility values are accepted.
Use at most 4096 position records and 64 KiB encoded visibility output per
section, checked before allocation; charge decoded storage against arismoko/horizon#1508's
32 MiB section and 128 MiB in-flight ceilings. Admission waits at the global
limit. Snapshot sharing counts storage once, leases remain charged until drop.

A bad result refuses that registration's entire section answer atomically.
Native unwind panics and wasm traps/fuel exhaustion disable the registration
for the generation, emit one diagnostic and invalidate dependent plans; rebuild
using valid defaults and remaining listeners. No partially applied masks.
Native abort cannot be recovered; native callbacks require the supported unwind
boundary and cannot be forcibly preempted. Wasm gets a finite batch fuel budget.
Teardown invalidates mod generation and drains leases before unloading code.

No subscriptions selects the existing mesher once at dispatch: zero extension
snapshot allocations, callbacks, listener scans or new per-face dynamic dispatch.
Subscribed decisions are batched once per section; inner loops read data only.
Benchmark acceptance targets less than 1% no-mod throughput regression; no
performance measurement is claimed by this documentation change.

## Acceptance matrix

Future implementation tests, not results of this note:

| Input | Expected value |
| --- | --- |
| Isolated opaque cube, Keep, mask 63 | 6 model quads |
| Same cube, mask 61 (up removed) | 5 model quads |
| Two adjacent opaque cubes, default | 10 exterior quads |
| One cube wholly hidden beside another | 0 hidden quads, 6 remaining cube quads |
| HideTerrain with fluid and one arismoko/horizon#864 quad | 0 terrain quads; independent block-entity draw retained |
| Hidden state in raw snapshot | Original state id unchanged |
| Border neighbour Missing, arrives, unloads | Exposed cube faces 6, then 5, then 6 |
| Write at local (15,15,15), neighbours loaded | Exactly 8 radius-one sections dirtied |
| Resource A completes after B; unload/reload same coordinate | 0 stale uploads in both cases |
| Two translucent quads at distances 2 and 5 | 2 translucent quads, distance 5 sorted first |
| Mask 64, duplicate position, 4097 records, panic or trap | Whole faulty answer refused; default cube 6 faces |
| Native and wasm same snapshot/answers | Identical masks, occlusion and face counts |
| No subscribers | Baseline mesh, 0 extension allocations and callbacks |

## Proposed stages

File none: do not file implementation issues until Ari posts the decision.
First approve granularity, raw input and occlusion semantics. Then implement
shared snapshot/epoch infrastructure with arismoko/horizon#1508, followed by generated native
and wasm preparation adapters together. Finally integrate arismoko/horizon#864 contribution
masking and exercise the route matrix, counters and throughput benchmark.

## Open questions

- Does section hiding include independently drawn block entities? Recommend no:
  preserve R2 lifecycle ownership and make a separate explicit lifecycle choice.
- Should masks force faces visible? Recommend suppression only; whole-cell hiding
  already exposes neighbours and avoids contradictory occlusion declarations.
- Can mods read beyond one block of halo? Recommend no, matching arismoko/horizon#1508; a future
  radius extension requires explicit dependency and admission budgets.
- Are the proposed output limits sufficient? Recommend 4096 records/64 KiB as
  initial ceilings and representative native/wasm measurements before raising them.
- What exact public name and refresh receiver should ship? Recommend one section
  visibility preparation contract and a natural client-world handle refresh,
  finalized with arismoko/horizon#1508 rather than a broad new Host service.
