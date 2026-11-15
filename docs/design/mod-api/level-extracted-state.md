# Design: level extracted state

Provisional answer to [arismoko/horizon#866](https://github.com/arismoko/horizon/issues/866):
use a typed, frame-owned extension store, freeze it after base extraction and
`extract_level_render_state`, and lend immutable values to drawing through a
snapshot-only `DrawContext`. This is a design proposal, not an implemented API
or a claim that the issue is complete.

## Vanilla and NeoForge 26.3

Verified with the pinned vanilla oracle from `/workspace/horizon`:
`python3 tools/rea_vanilla.py class net.minecraft.client.renderer.LevelRenderer`
and `python3 tools/rea_vanilla.py method net.minecraft.client.renderer.LevelRenderer render`.
`render` passes `levelRenderState` to `submitFeatures`, then calls
`FeatureRenderDispatcher.prepareFrame` before constructing the frame graph.
This verifies a state-to-submission boundary, not arbitrary mod storage or a
particular extraction thread. A query for `extractLevel` failed because that
method is absent; the complete extraction caller/thread census remains open.
Vanilla has no mod registration contract.

The issue names NeoForge 26.3 `ExtractLevelRenderStateEvent` as the counterpart.
The public [26.3 Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/client/event/ExtractLevelRenderStateEvent.html)
was inaccessible during this investigation. Its exact fields, cancellation,
thread guarantees, ordering and attachment limits are **unverified**. The issue's
description is intent, not verification of NeoForge behavior. Require reachable
public documentation before adopting the counterpart's full contract; do not
infer it from older versions, source or patches. All limits below are proposed
Horizon limits, not NeoForge limits.

## Verified Horizon owners and missing seam

Paths below are relative to the repository at `731b0cae68`.

| Owner | Evidence | Implication |
| --- | --- | --- |
| Frame preparation | `crates/horizon-client/src/game.rs:15048`, `render_with` | Builds frame before consumer; proposed extraction/freeze integration belongs here |
| Renderer consumer | `crates/horizon-client/src/game.rs:15040`, `render_frame` | Calls renderer with prepared frame |
| Deferred submission | `crates/horizon-client/src/game.rs:15029` | Submit thread may retain a frame after presentation |
| Frame fields | `crates/horizon-client-render/src/gfx.rs:1104`, `Frame` | Existing borrowed frame is not proof of owned extension storage |
| Renderer entry and resize | `crates/horizon-client-render/src/gfx.rs:8024`, `:5640` | Submission and target replacement need explicit epoch checks |
| Renderer replacement | `crates/horizon-client/src/game.rs:14236`, `adopt_renderer` | Rebuild must retire old extension/resource generations |
| Fog extraction | `crates/horizon-client/src/game/interface/render_tick.rs:133` | Existing base preparation must precede extension freeze |
| Block-screen visibility | `crates/horizon-client/src/game/interface/render_tick.rs:529` | Keep existing extraction/cancellation decisions; do not rerun in draw |
| Draw adapter | `crates/horizon-client/src/game/host/draw.rs:6` | Currently contains `&Game`; cannot certify snapshot-only access |

This is a verified integration census, not a complete proof of every submission,
reload or early-return route. Implementation must audit normal world rendering,
screenshot sequences, split views, loading/title frames, device loss and deferred
submission. Early loading returns inside `render_with` must not fire a world hook.

## Proposed contract

Register `LevelStateKey<T>` at mod load, resolve it to a dense slot and retain
schema and mod-generation identity. `T` must use an API-owned `LevelStateWire`
schema: fixed-width scalars, bounded sequences/strings, API enums, vectors and
resolved resource keys. The API owns schema validation and generated native/wasm
encoding. No `Any`, Rust pointers, engine structs, closures, GPU handles or live
entity/world references cross this boundary. Custom records compose supported
Wire fields; a type parameter is not permission to serialize arbitrary memory.

Registration returns the named refusal `UnsupportedLevelStateCapability` for
unsupported field kinds, live references, custom destructors or requested mutable
draw access; duplicate keys return `DuplicateLevelStateKey`, invalid bounds return
`InvalidLevelStateSchema`. Refuse before enabling the listener. These refusals
apply only to this registrar's declarations, not unrelated unknown identifiers.

On the client owner thread: capture base camera, fog and existing visibility
results; decide the world/view and epoch; fire one dependency-ordered
`extract_level_render_state` with `ClientHost` and a mutable typed frame writer;
validate and atomically publish each listener's contribution; freeze; submit.
Host writes and apply use the existing transaction discipline. Recheck world,
resource and mod generations after callbacks; refuse stale publication without
recomputing or firing twice. Extraction receives no `DrawContext`.

The frame packet owns the frozen store, identified by frame sequence, view,
world lifetime, resource generation, target generation and mod generation.
Values default to absent every frame; no inheritance from frame N to N+1.
A listener may write only its registered slots; declared dependent readers may
read those slots. Missing values return `None`. Later listeners see only committed
prior contributions. No draw callback mutates the store.

Drawing receives a frame-scoped read view through `DrawContext` and reads
`state(key) -> Option<&T>` (equivalent checked decoded view on wasm). The context
exposes captured camera/resources and approved draw commands, never a route to
mutable live game state. Existing adapters holding `Game` require refactoring
before this promise can ship. Draw callbacks run during command preparation;
submission workers consume immutable commands and leases, and never call mods
or carry a `ClientHost`. This permits separate extraction/submission threads
without requiring native or wasm callbacks to be worker-safe.

A frame and every deferred consumer share an immutable lease. Release only after
the final CPU consumer completes; GPU-referenced resources retain their existing
fence lifetime separately. Never clear or reuse storage at flip/present alone.
Bound in-flight packets to 3; backpressure before extraction when all are leased.
Suspended/zero-size/no-world frames create no level store and fire no level hook.
Failed or cancelled frames release their lease and cannot publish commands.

Resize increments target generation: refuse old target-dependent submission and
extract afresh; never patch frozen dimensions. Resource reload and dimension
switch increment their respective generations, retire old packets and prevent
new publication from them. Existing accepted GPU work may finish with leased old
resources. Teardown stops callbacks, drains/cancels CPU submissions, waits for
resource users and then releases stores before unloading native code/wasm instances.

## Untrusted input and unused cost

Proposed ceilings: 256 slots per mod, 64 KiB encoded and decoded per slot,
1 MiB per mod/frame, 8 MiB total/frame, sequence length 4096, nesting depth 8.
Use checked arithmetic and validate lengths before allocating. Floats must be
finite; camera-relative positions have magnitude at most 1,000,000, alpha is
[0,1], and resource keys must match the captured generation. Reject invalid tags,
NaN, infinity, stale keys and oversized values rather than clamp silently.

Invalid output rolls back that listener's entire contribution; built-in state
and earlier valid contributions survive. Catch native unwinds at the adapter;
wasm traps/fuel exhaustion have the same policy. Disable the faulty listener for
its mod generation and emit one diagnostic. Native process aborts are not
recoverable; native callbacks cannot be safely forcibly preempted. Wasm has a
finite callback fuel budget. No arbitrary mod destructor executes on submission.

Resolve subscriptions and slots at registration. With no extraction listeners
and no state consumers, one cached enabled-bit branch selects the existing path:
0 extension allocations, 0 store clears, 0 serialization, 0 callbacks and 0 scans
per frame. With listeners, reuse bounded arenas only after leases expire; dispatch
only subscribed listeners. Verify allocation counters and benchmark the callee
path before claiming a measured performance result.

## Acceptance matrix

Future implementation tests; these are expected values, not passing results.

| Scenario | Concrete expected result |
| --- | --- |
| Extract A=7; live value changes to B=9 before draw | N draws 7; N+1 extracts/draws 9 |
| N writes 7; listener omits slot in N+1 | N+1 reads None |
| Two views, contributions 3 and 5 | Each draws its own value, never the other's |
| Listener order dependency A then B | B reads A's committed 7 exactly once |
| Base overlay cancelled before level hook | 0 built-in overlay draws after freeze |
| NaN, infinity, 65537-byte slot, 4097 elements | 0 offending listener values; built-in state preserved |
| Native unwind / wasm trap after writing 7 | Slot absent; 1 diagnostic; 0 later callbacks that generation |
| Reload epoch 4 to 5 before publication | 0 epoch-4 publications into epoch 5 |
| Resize 800x600 to 1024x768 | New snapshot reports 1024x768; 0 old-target submissions |
| No world / zero-size surface | 0 level callbacks and stores |
| Three retained packets; request fourth | 3 live stores; fourth waits before extraction |
| Present N while deferred consumer holds N | N still reads 7; arena reuse count 0 until release |
| Same Wire input native/wasm | Same slot values, refusal and ordering results |
| No subscriptions for 100 frames | 0 extension allocations, scans and callbacks |

## Proposed stages

Implementation file: none in this change. First approve ownership, Wire schema,
limits and public counterpart evidence. Next audit and refactor the complete
frame/draw boundary, with lifetime and early-return tests. Then add the catalog
hook, generated native/wasm adapters and typed storage together. Finally wire a
sample contribution into a real drawing consumer and run the matrix plus empty
path allocation and throughput measurements. No Rust API is added by this note.

## Open questions

- Exact NeoForge fields, ordering, cancellation and limits are unverified.
  Recommend requiring public 26.3 Javadoc before final hook adoption; keep the
  proposed store as Horizon design, not a claim of counterpart equivalence.
- Complete extraction/caller and async owner census remains unfinished.
  Recommend proving all world/view entry and early-return routes before coding
  dispatch, using vanilla oracle callers and Horizon consumer tests.
- How broad should snapshot-only DrawContext migration be? Recommend migrating
  every consumer of this store and refusing registrations requiring live reads;
  do not expose the store through the current Game-backed adapter unchanged.
- Are budgets sufficient for real mods? Recommend the explicit ceilings above
  initially, measure representative native/wasm scenes, and change them only
  through reviewed schema policy. Do not offer an unbounded native escape hatch.
