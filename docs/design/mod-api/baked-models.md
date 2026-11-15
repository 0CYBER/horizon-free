# Design: baked models and ModelData

Provisional answer to [arismoko/horizon#861](https://github.com/arismoko/horizon/issues/861):
the engine owns immutable modified baked results for each resource generation;
mods own bounded edit descriptions, never mutable renderer models. Evaluate
ModelData on the client world-owning thread before freezing geometry selection.
Workers read concrete results from [section meshing](section-meshing.md)'s arismoko/horizon#1508
plan. This extends that plan's input boundary; its scheduling, halo, publication,
geometry limits and rendering rules remain authoritative. No Rust API is
implemented; every stage below has file **none** pending design approval.

## Evidence and limits

Vanilla 26.3 was inspected only through `tools/rea_vanilla.py` from
`/workspace/horizon`: `method net.minecraft.client.renderer.chunk.SectionCompiler
compile`, its `callers` query, and `method
net.minecraft.client.renderer.item.ItemModelResolver updateForTopItem`.
SectionCompiler reads a render region, selects block models by state, passes a
position seed to tessellation, treats fluids and block entities separately, and
sorts translucent output. Its verified caller is CompileTask.doTask. Item
resolution clears output and appends layers for nonempty stacks with display
context, level, owner and seed. These observations do not establish a vanilla
ModelData API or a vanilla mod callback. The owner-thread preparation boundary
below is Horizon policy.

The public [NeoForge model guide](https://docs.neoforged.net/docs/resources/client/models/modelsystem/)
was reachable on 2026-11-09 but labelled 26.1. It describes client baking-result
edits for block and item models and block-entity ModelData consumed during model
selection. It is contextual evidence only. The requested
[26.3 ModifyBakingResult Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/client/event/ModelEvent.ModifyBakingResult.html)
was inaccessible. Exact 26.3 event threading, mutability, reload ordering,
ModelData invalidation and item applicability are **unverified**. In particular,
no claimed NeoForge radius, size, thread-safety or item-data limit is verified.
Require public 26.3 documentation of those contracts before adopting counterpart
semantics or implementing the hook. No NeoForge source/patches or other project's
source were consulted.

## Ownership and registration

Propose one client `modify_baking_result` moment, with `ClientHost`, after baking
and before publishing caches on initial load and every resource reload. Client-lib
prepares the candidate; game dispatches subscribers in dependency order; validation
and publication apply in the same transaction, rechecking resource/mod generations.
Refuse stale preconditions without recomputing inside apply. Resource preparation
may be asynchronous, but dispatch and commit return to the client owner thread.
Keep standalone registration unchanged.

API-owned proposed representations: `BakedModelKeyWire` (resource generation,
resolved slot and block/item kind), `BakedModelEditWire`, `BakedModelRecipeWire`,
`ModelDataSchemaWire`, `ModelDataValueWire`, `ModelEvaluationInputWire`, and
`ModelEvaluationResultWire`. Reuse arismoko/horizon#1508's prepared model/material/quad Wire
values rather than introduce another geometry format. Recipes select bounded
prepared variants or validated replacement geometry, with explicit particle,
lighting, culling and item transform metadata. Wire is a mod boundary, not network
traffic. No renderer structs, pointers, closures or GPU objects cross it.

Resolve model targets and schema property ids once at registration. Properties
have declared scalar/enum or bounded-vector types and defaults; no arbitrary
serialized objects. Refuse unsupported capabilities at registration with owner
and capability named: worker callbacks, live-world evaluation during meshing,
unbounded neighbourhoods, arbitrary native model delegates, raw pipelines and
undeclared data types. Apply the same refusal on native and wasm. Reject conflicting
exclusive replacements with both owners named; composable edits run in dependency
order. Unknown feature-owned target ids refuse registration; unrelated existing
unknown-id no-ops retain their behavior.

Build generation B from B's original baked inputs, never from modified A. Publish
block and item tables atomically. Shared immutable engine references lease A until
old jobs and item draw snapshots release it; A cannot publish into B. Mods receive
scoped keys, not ownership of engine allocations. Teardown follows arismoko/horizon#1508's job
lease/drain ordering before unloading native code or wasm stores.

## ModelData evaluation boundary and routes

Registered providers receive a read-only captured evaluation input plus
`ClientHost` at owner-thread preparation. Their result is a typed property value
set, not a callback stored in the model. World writes are forbidden during this
preparation moment. Block inputs use arismoko/horizon#1508's radius-one snapshot and explicit
Missing cells; model selection freezes deterministic seeds, data revision,
resource generation and prepared geometry keys into state recipes/sparse overrides.
The mesher receives those through the existing proposed SectionMeshPlanWire.
There is **no worker hot-path dispatch**, native trait-object invocation or wasm
call for this feature. Existing worker tint capabilities remain as documented in
section meshing; they do not authorize ModelData callbacks.

Cache by model key, typed input revisions and seed. Block/state, neighbour,
block-entity data, light/biome dependencies and provider invalidations dirty the
applicable plans; never poll arbitrary mod state per frame. Reload invalidates all
old data keys. Providers explicitly request invalidation on changed source data.
Missing data uses declared defaults. Stale data refuses publication using arismoko/horizon#1508's
tokens. Items evaluate during owner-thread item preparation using captured stack
components, display context, optional level/owner and seed; freeze output before
drawing. A block item does not inherit world-position ModelData implicitly.

Verified current integration owners at base `0ef42f9e54` (paths relative to repo;
these are seams, not already implemented callbacks):

| Route | Owner | Proposed integration |
| --- | --- | --- |
| Block library construction | `crates/horizon-client-render/src/model_library.rs:64`, `:82` | Produce candidate block view before modified cache publication |
| Item library construction | `crates/horizon-client-render/src/model_library.rs:120` | Produce item view in same generation |
| Standalone additions and block preparation | `crates/horizon-client-render/src/models.rs:37`, `:142` | Preserve additions; edit baked results after preparation |
| Terrain model consumption | `crates/horizon-client-render/src/mesher.rs:1840` | Read frozen model parts; no provider calls |
| Item bounds and draw submission | `crates/horizon-client/src/item_render.rs:229`, `:267`, `:299` | Consume prepared item geometry including skinned submission |

Initial mesh, remesh, halo changes, unload, reload, cancellation and stale upload
all use section meshing's route and token table, not a second invalidation system.
GUI, held, dropped, frame and equipment item callers must converge before item
submission; exact preparation caller coverage remains to be audited before code.
Special block-entity drawing stays on R2; it may consume a prepared model but does
not acquire a terrain callback. Dedicated servers neither bake nor evaluate.

## Untrusted input and cost

Proposed ceilings: 64 properties per schema, 256 elements per vector, 16 KiB
encoded and decoded ModelData per evaluation, 256 variants per recipe. Checked
length arithmetic precedes allocation; count captured values against arismoko/horizon#1508's
32 MiB section and 128 MiB in-flight budgets. Geometry obeys that page's quad,
coordinate, UV, colour and material bounds. Reject NaN/infinity, invalid tags,
stale keys, duplicate properties and values outside declared scalar ranges before
selection. No partial edits: refusal preserves the original candidate model.

Catch unwind-capable native panics at the adapter; wasm traps or fuel exhaustion
have identical refusal. Disable the faulty registration for this generation,
diagnose once and invalidate its cached selections; use original baked geometry
and default data. Native abort cannot be caught and native callbacks cannot be
safely forcibly preempted. Wasm gets finite fuel and the same output limits.
Generated adapters share semantics and dependency ordering across both targets.

With no subscribers, choose the existing path once at resource/dispatch setup:
zero ModelData allocations, provider calls, per-block checks or all-mod scans.
With subscribers, dispatch only indexed providers during preparation; workers read
arrays. Reload edits cost only at bake time. Performance numbers remain unmeasured.

## Acceptance matrix

Future implementation tests, not results of this docs change:

| Input | Concrete expected value |
| --- | --- |
| Six-quad model changed to one quad, first load then reload | 1 quad on both; edit called once per generation, not twice cumulatively |
| Untouched six-quad model | 6 quads before and after reload |
| Boolean property false/true selects variants 10/11 | Default false gives 10; update true gives 11 after invalidation |
| Same item in GUI and ground with declared context variants 20/21 | GUI 20; ground 21; no implicit block-position data |
| A job finishes after B reload | 0 A uploads; B model remains selected |
| Data vector length 257 or payload 16385 bytes | Whole result refused; default false selects 10 |
| NaN, infinity, stale material key, panic or wasm trap | No partial edit; original cube has 6 quads |
| Identical native/wasm inputs and seed | Same property values, model 11 and quad/material sequence |
| Worker consumes 4096-cell plan | 0 ModelData callback invocations on worker |
| No registrations | 0 extension allocations and callbacks; baseline geometry identical |
| Registration requests live worker evaluation | Refused before any job is scheduled |

## Proposed stages

1. Public-contract audit and approval (file none): obtain exact 26.3 documentation,
   settle counterpart semantics and audit every item preparation caller.
2. Shared boundary and generation ownership (file none): approve API-owned Wire
   types, registration refusals and native/wasm codecs together.
3. Baking and owner-thread evaluation (file none): implement one transactional
   moment, reload leases and provider invalidation with focused fixtures.
4. Frozen-plan and item integration (file none): wire arismoko/horizon#1508 and item preparation,
   then run the matrix and measure empty-path cost before enabling registration.

## Open questions

- Exact NeoForge 26.3 contract? Recommend requiring public documentation before
  naming the final hook contract; do not infer restrictions from 26.1 or source.
- Arbitrary dynamic delegates versus finite variants? Recommend finite recipes
  and owner-prepared geometry; refuse unsupported registrations explicitly.
- Item ModelData parity? Recommend a distinct typed item evaluation input under
  the same preparation contract, conditional on documented counterpart coverage.
- Final R2/#1508 geometry identities? Recommend reuse of their approved keys and
  generation leases rather than a parallel model store.
- Data budgets and provider scheduling cost? Recommend the explicit ceilings above
  initially, with dense-scene measurements before raising them.
- Complete item caller convergence? Recommend an implementation gate: audit GUI,
  hands, dropped items, frames and equipment and test context-dependent selection
  on each before claiming full route coverage.
