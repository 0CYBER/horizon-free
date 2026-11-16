# Design: per-type batched block entity ticks

**Provisional; implementation waits for a maintainer decision.** This answers
[arismoko/horizon#2759](https://github.com/arismoko/horizon/issues/2759).
The recommendation is contiguous same-type runs in the existing ticker order,
with each entity committed before the next one starts. This reduces outer calls
where runs exist; it does not promise one call per type across a whole level.

**Implemented separately in arismoko/horizon#3669:** the existing individual tick now has
`BlockEntityTickReply::{Changed, Unchanged}` on native and wasm. Set
`*event.reply = BlockEntityTickReply::Unchanged` to omit that Part's reply snapshot
entirely. Each Part starts with `Changed` and sees only preceding accepted edits;
an `Unchanged` Part cannot leak its attempted edits or erase an earlier `Changed`.
If all Parts answer `Unchanged`, there is no snapshot decode/application, revision
increment, tick-only dirty mark or container/comparator notification. The original
entity is restored only if its original placeholder remains. Independent Host writes
keep their effects; aborted Host transactions stay aborted. Not choosing this form
retains full checked `Changed` write-back. Input snapshots are still sent; runs,
deltas, `NeedFull` and validity caches remain unimplemented.

Audit inputs for this implemented subset (not evidence of batching performance):

| Owner | Audited behavior / executable input |
| --- | --- |
| `crates/horizon-mod-vocab/src/block/types.rs`, `hooks/block_entity_tick.rs` | One reply model and the existing execution hook; default `Changed`. |
| `crates/horizon-mod-api-gen/src/dispatch.rs`, `ModBlockEntityView` | Per-Part reply reset and acceptance; native payload rollback is captured lazily on the first actual mutation, not on an untouched Unchanged call. |
| `crates/horizon-mod-api-gen/src/wire/calls.rs`, `wire/host.rs` | Reply discriminator precedes optional snapshot bytes; Unchanged omits both snapshot encode and decode/apply, with no legacy wire fallback. |
| `crates/horizon-client/src/game/native_hooks/block_entity_tick.rs` | Original placeholder identity checked before unchanged restoration; no dirty/effect owner called. |
| `crates/horizon-client/src/game/modding/block_entity_tick_reply.rs`, `block_entity_tick_items.rs` | Changed reuses checked write-back; independent dropped-item Host proposals retain their existing acceptance checks even on Unchanged. |
| `crates/horizon-client/src/game/modding/block_entity_tick_reply_tests.rs` and `crates/horizon-mod-wasm/tests/fixture/src/tick_reply.rs` | Shared native/compiled-wasm Parts: repeated clean constant-revision ticks, Unchanged→Changed and Changed→Unchanged transitions, default reply reset, aborted item transactions with unnormalized original slots, accepted Host writes, replacement, and container/comparator callback counts. `wasm-mods.yml` owns its compiled-fixture prerequisites; ordinary prepared-data shards exclude it. |

## Existing moments and verified owners

The existing plural `block_entity_ticks` is **not a batched execution hook**.
`crates/horizon-mod-vocab/src/hooks/block_entity_ticks.rs:4` declares a server,
read-Host eligibility query, keyed by block, asked for block states at world open.
`BlockEntityTicksEvent.ticks` selects whether a ticker exists; it contains no list
of entities. The Part example at
`crates/horizon-mod-vocab/src/parts/block_entities.rs:334` sets it to true, then
implements the separate mutable individual tick. The adapter is
`crates/horizon-client/src/game/native_hooks/base.rs:1083`, called by
`crates/horizon-client/src/game/native_hooks/shared.rs:3112` (`native_ticks`).
Reuse this eligibility decision and its frozen state tables. Do not turn it into
a second execution callback, or introduce a parallel batch moment.

Extend the existing singular execution moment instead:
`crates/horizon-mod-vocab/src/hooks/block_entity_tick.rs:4` and
`crates/horizon-client/src/game/native_hooks/block_entity_tick.rs`.
Replace its individual event/dispatch shape with the approved run cursor shape
in one clean break, updating all callers, generated catalog/coverage, examples
and the machine sample together. No legacy individual callback or compatibility
adapter remains alongside it. A run of length one uses the same listener.
It remains a `BlockEntityPart` listener with `&mut dyn ServerHost` on both targets.

| Route / owner on inspected main | Integration requirement |
| --- | --- |
| `crates/horizon-client/src/game.rs:8564` | Server world pass calls the common ticker loop; integrated and dedicated execution use this route. Remote clients do not run authoritative saved-data ticks. |
| `crates/horizon-client-render/src/world.rs:502` | Supplies the ordered ticker snapshot. Do not regroup the level by type. |
| `crates/horizon-client-render/src/block_entities.rs:750,787` | Pending additions enter the list at snapshot acquisition; validate each live slot immediately before use. Ticker slots are not durable holder identities. |
| `crates/horizon-client/src/game/blocks/block_entity_ticks.rs:45,60,91` | Preserve chunk ticking, sleep, liveness and border checks; replace only the mod-entity dispatch branch. Built-in ticks remain at their existing positions. |
| `crates/horizon-client/src/game/native_hooks/block_entity_tick.rs`, `native_hooks/shared.rs::put_back_mod_block_entity` | Individual take/placeholder/restore owner: Unchanged already avoids dirty/effect owners and checks original placeholder identity. Changed retains the existing restore/effect owner. Batched holder-safe per-entry processing remains future work. |
| `crates/horizon-client/src/game/modding/block_entity_tick_reply.rs`, `modding/api_values.rs` | API conversion and checked write-back owners; reuse validation rather than adding an unchecked delta route. |
| `crates/horizon-mod-api-gen/src/dispatch.rs:280,447` | Existing Part dispatch and generated Wire event machinery own the new shape for both targets. |
| `crates/horizon-mod-api-gen/dispatch-tests/src/fixtures/stateful_block_entity.rs:1` | Existing factory/eligibility fixture is a model for dispatch ownership, not evidence of batched execution. Extend generated fixtures for the new execution shape. |
| `crates/horizon-client/src/game/blocks/spill.rs:1303` | Direct tick invocation is in a regression test. Update it to execute a singleton run through the same owner. |

The loop also invokes built-in hoppers, furnaces and other vanilla tickers.
Separate sculk passes (`crates/horizon-client/src/game.rs:8565`) and client
animation work are outside this mod saved-data listener; do not move them into
it. Implementation must audit all generated callers and any newly added routes.
These anchors identify present owners, not already implemented guarantees.

## Vanilla and NeoForge evidence

Inspect the pinned vanilla 26.3 bytecode reproducibly with:

```sh
python3 tools/rea_vanilla.py method net.minecraft.world.level.Level tickBlockEntities
python3 tools/rea_vanilla.py method net.minecraft.world.level.chunk.LevelChunk.BoundTickingBlockEntity tick
python3 tools/rea_vanilla.py method net.minecraft.world.level.chunk.LevelChunk.RebindableTickingBlockEntityWrapper tick
```

`Level.tickBlockEntities` appends pending tickers before iteration, removes
removed entries as it reaches them, and otherwise calls each eligible ticker in
list order when normal ticking and `shouldTickBlocksAt` permit it. New pending
entries added during iteration wait for a later pass. The bound ticker checks
removal, level presence, chunk ticking and the current block state's validity
before invoking the ticker. Exceptions produce a ticking-block-entity crash
report. The rebindable wrapper’s `tick` delegates to its current ticker. These
observations support ordered processing and fresh checks, not a whole-level
type grouping. The actual `rebind` and replacement callers remain unverified
and must be checked before implementing replacement parity; a replacement must never receive an old
entity's saved-data reply.

The public [NeoForge block entity guide](https://docs.neoforged.net/docs/blockentities/#tickers)
retrieved for this note is labelled **26.1**. It describes `EntityBlock#getTicker`
returning a per-entity `BlockEntityTicker` and recommends caching expensive work.
Its [storage section](https://docs.neoforged.net/docs/blockentities/#storing-data)
requires `setChanged` to mark the chunk for saving. This is contextual evidence,
not a 26.3 batching contract. The 26.3
`BlockEntityTicker` javadoc was inaccessible. NeoForge 26.3 batching, limits and fault behavior remain
**unverified**; no counterpart or equivalence is claimed. Only public docs and
Javadoc were consulted; no NeoForge source or patches were read.

## Proposed execution contract

Choose **A: contiguous runs**, not B: one call per type across the level.
For `[A1,A2,B1,A3]`, dispatch `[A1,A2]`, tick B1, then dispatch `[A3]`.
An ineligible or different-type slot ends a run; do not scan ahead to merge runs.
Grouping by type would move A3 before B1 and change neighbor reads, hopper
chains and observable side effects. There is no batching measurement justifying
that change. Preserve dependency order of listeners **within each entity**:
A1 listener L1, A1 L2, A2 L1, A2 L2. A naive outer call to L1 for all entries
followed by L2 for all entries violates this rule. Initially batch only runs
with one resolved execution listener; with multiple listeners, use singleton
runs through the same cursor contract until a scheduler preserves that order.

A precomputed list of snapshots and an eventual list of replies cannot provide
per-entity visibility. Recommend a host-driven, sequential cursor: the outer
listener receives the run context, asks for its next entry, processes it, then
calls a checked `done(result)` operation before requesting another entry.
`done` validates and completes that entity's action, and only then may `next`
materialize the next snapshot. This import costs a crossing per entity in wasm;
benchmark the net saving rather than advertising elimination of all crossings.
Only one entry may be outstanding; double completion, wrong run tokens and
out-of-order identities refuse without mutation. Ending a call with an
unfinished entry aborts that entry; it must not silently mark it complete.

Each snapshot carries the existing scoped lifecycle holder, type key, position,
current block state and saved-data revision. The revision is not the lifecycle
generation. Read it beside the holder; cache reuse requires equality of holder,
revision, position and state. Recommend a full saved-data snapshot on cache miss
and a compact validity record on hit. `NeedFull` supplies that same entry's full
snapshot once without advancing the cursor or executing a second tick. Recheck
versions after host writes; a stale reply refuses rather than recomputing or
writing over another entity. The saved-data revision owner is unresolved: arismoko/horizon#2794 is the outside-engine
JVM/language-host workstream, not this prerequisite. Identify the actual revision
owner and audit every saved-data and inventory mutation path before implementation;
cache reuse is gated on complete mutation coverage.

The entity action covers listener Host writes and checked saved-data apply.
A1's commit is visible to A2, including neighbor reads. Do not hold a mutable
engine entity borrow across dispatch or remove the entire run into placeholders.
Revalidate eligibility, residency, holder and state at each `next` and `done`.
A1 removing A2 means no A2 callback; unloading its chunk means no subsequent
callback there. A replacement invalidates the old reply and cache match. New
pending tickers wait until the next pass; wrapper rebinding must preserve the
vanilla distinction between updating a ticker and appending a new one.

Reuse `crates/horizon-client/src/game/runtime/transactions.rs:193,212,293`
for action savepoints and block-entity journaling, and the
[Part instance state contract](part-instance-state.md) for holder identity and
state. During lifted outer actions, per-entry commit merges into the parent:
later entries see tentative earlier writes, but retirement, irreversible effects
and journal publication wait for root commit; root abort restores all entries.
Do not open a batch-wide transaction in the normal tick path. Refuse saves while
an action savepoint is open, so today's temporary placeholder cannot omit an
entity from a save. This gate and holder-safe access are prerequisites, not
claims about the current singular callback.

## Write-back, caches and trust

The individual `Unchanged` result is implemented above; batching's proposed results are `Unchanged`, a validated delta, or `NeedFull`. Deltas contain changed
and removed saved keys plus checked item/data-slot updates; they cannot edit
identity or type. Reject duplicate/conflicting keys, invalid slots/stacks,
wrong holders, stale revisions and malformed Wire before any apply. Apply the
whole entity result atomically through the existing write-back validation.
Deleting absent key K or setting V=5 when V is already 5 is unchanged: no dirty
mark solely for ticking or receiving a delta. A real write marks the owning
chunk and emits required container/comparator effects once through existing
owners. A listener's independent Host write still has its own normal dirty
effects. The individual Unchanged path implements the first piece of
[arismoko/horizon#1780](https://github.com/arismoko/horizon/issues/1780); delta/caching semantics remain proposed.

API-owned Wire records must represent cursor tokens, holder/revision metadata,
full snapshots and results; no engine NBT types, guest pointers or native-only
`Any` cross the boundary. Native and wasm expose the same listener, ordering,
snapshot fields, validation and failure outcomes. Unsupported record/state Wire
capabilities refuse registration/load with the proposed named refusal
`UnsupportedBatchTickCapability(type, capability)` before ticking.

Bound lengths, total encoded bytes, nesting, cursor lifetime and host operations
using shared Wire/NBT and runtime fuel/deadline budgets on both targets. Require
at most one full-snapshot retry per entry. Chunk long runs at existing transport
limits without reordering; batch capacity must not cap the world's vanilla
entity count. A full snapshot too large for one frame needs the approved shared
bounded transfer route, never truncation or silently skipping that entity.
Exact numeric budgets and that route require approval before code.

On native unwinding or a wasm trap/interrupt, abort the outstanding entity's
savepoint and invalidate that cache entry. Keep already root-committed A1.
Use the runtime's existing fault policy to disable/stop the failing listener;
do not retry it later in this run. Other listeners and built-in entries follow
the existing dispatch policy. Process-aborting native panics are unrecoverable.
Wasm stop/disable owners include `crates/horizon-mod-wasm/src/runtime.rs:426,631`;
precise native recovery and later-listener continuation need an explicit audit
before implementation claims parity. A lifted root abort can still undo A1.

A mod may keep a bounded, evictable cache in its own memory, never saved or synced;
losing it only causes a rebuild. Persistent typed per-holder data must use
arismoko/horizon#796/#781's attachment-backed contract. No new engine cache store or generation
is introduced. Invalidation notices may be an optimization of existing holder
lifecycle notices, never necessary for correctness. A 4096-entry LRU is the
external experiment's policy, not an engine or vanilla limit.

## Cost and measurement input

Resolve subscribers by type at registration. With no subscriber, one O(1)
type-table lookup selects today's path: zero batch events, no grouping, no
entity snapshots/copies and zero allocations beyond today's ticker machinery.
Do not scan every mod or serialize data to discover whether someone listens.
Subscribed cost is linear in processed entries and changed bytes, with bounded
buffer reuse; alternating types may offer no batching gain.

[reservedbytes' measurements on arismoko/horizon#2759](https://github.com/arismoko/horizon/issues/2759)
are external input, **not re-measured here**. Their 1000 cooking entities ran on
a Ryzen 9 9950X, Ubuntu 26.04, pinned to eight physical cores on a loaded box
(load 40–58), fork `964de9d3f`, Wasmtime 50.0.1, a custom single-store runtime
rather than the group linker, four interleaved runs per side. Retaining guest
objects changed tick mean 125.3 to 94.4 ms (median across runs), guest handler
103.9 to 74.6 ms, and collection 7.6 to 13.5 ms per tick. Earlier-build bytes
in/out changed 371/287 to 21/24 per entity; in-game host calls 46 to 33, offline
54 to 39. Offline interpreter work changed 774k to 591k operations; full rebuild
cost about 260k and delta comparison added about 127k. Post-collection live heap
rose 1.1 to 6.2 MB. **All results still used one call per entity; batching was
not measured.** They motivate cache experiments and GC accounting, not a claimed
Horizon batching speedup.

Existing measurements are
`crates/horizon-mod-wasm/tests/bench.rs:84` (1000 fake in-memory dispatches) and
`crates/horizon-client/src/game/modding/native_live_holder_tests.rs:46` (real
Game holder path). Neither establishes this proposed batching's performance.
Measure no-listener, singleton, long-run and alternating-type scenes on both
targets, including collection, memory, payload bytes, host calls and multi-listener
order. Do not promise an improvement until those results include the cursor cost.

Batching ships only with a measured total tick win on a real prepared-data scene
in the optimized profile, including cursor crossings and GC. The no-mod tick must
remain within measurement noise, and the build must change no saved data.
These are shipping conditions, not optional performance claims.

## Acceptance matrix

These are proposed implementation tests with literal expectations, not checks
executed for this documentation-only change.

| Fixture | Expected result |
| --- | --- |
| `[A1,A2,B1,A3]`, only A subscribed | Two outer calls: `[A1,A2]`, then `[A3]`; B1 ticks once between them. |
| A1 commits V=5; A2 reads V in the same run | A2 reads 5, not the pre-run value 0. |
| A1 removes A2 before `next` | A2 callbacks=0; no A2 snapshot/reply applied. |
| A1 unloads A2's chunk | A2 callbacks=0; batch does not load or pin the chunk. |
| Old A2 holder replaced, late old reply sets V=9 | Refusal; successor's initial V=0 remains 0; old cache cannot match. |
| A1 adds A4 during iteration | A4 callbacks this pass=0; next eligible pass=1. |
| Saved V=5; Unchanged, then delta V=5, absent-key deletion | Zero new dirty marks or comparator notices from these results. |
| Saved V=0; delta V=5 and remove existing K | Saved V=5; K absent; owning chunk marked once. |
| No subscriber, 1000 entities | 0 batch events; 0 extra allocations, grouping passes or serialized bytes. |
| A2 traps after A1 commits V=5 | A1 remains 5; A2 has 0 committed writes; no later invocation of failed listener in this run; built-ins follow runtime policy. |
| Lifted root starts V=0, A1 completes V=5, root aborts | V=0; 0 published retirement/sync effects. Save while open refuses. |
| L1 and L2 subscribe to A1/A2 | Singleton scheduling yields `[A1:L1,A1:L2,A2:L1,A2:L2]`. |
| NeedFull twice, duplicate done, wrong token or bad slot | Refusal; entry advances 0 times until valid completion; saved V=0 stays 0. |
| Native and wasm process identical valid delta V=5 | Identical saved V=5, dirty flags and listener order. |
| Missing Wire capability on type A | Registration refuses with `UnsupportedBatchTickCapability(A, capability)`; 0 ticks. |

## Proposed implementation stages

The individual Unchanged subset is implemented; this note adds no batching runtime or benchmark claim.

1. Approve ordered runs, cursor/per-entity commits and multi-listener scheduling;
   complete wrapper and public NeoForge 26.3 verification.
2. Audit revision mutation coverage, holder-safe live access, save refusal,
   rollback and fault continuation with the lifecycle/write-back owners.
3. Specify API-owned Wire records and budgets; update the existing singular
   hook, generator, adapters, all callers and coverage in one clean break while
   retaining the plural eligibility moment.
4. Implement and test native/wasm matrix cases through the real holders and
   transaction owners; integrate changed-only apply with arismoko/horizon#1780.
5. Measure both existing benchmark paths and real ordered world scenes,
   including GC and unused-path costs, before claiming batching support.

## Open questions

- **Execution shape:** approve the cursor and singleton scheduling for multiple
  listeners. Recommendation: preserve exact order even at reduced batching gain;
  an eventual reply list cannot satisfy immediate visibility.
- **Revision completeness:** identify the actual saved-data revision owner and coordinate arismoko/horizon#1780;
  arismoko/horizon#2794 does not own revisions. Recommendation: require
  saved-data/items revision coverage and atomic checked apply before enabling
  cache reuse; never treat generation as a revision.
- **Replacement and faults:** confirm wrapper rebinding and native recovery,
  plus whether remaining healthy listeners continue after a fault.
  Recommendation: use the existing lifecycle and runtime policy, with no retry
  of a stopped listener and no stale successor writes.
- **Transport capacity:** approve numeric shared limits and full-snapshot transfer.
  Recommendation: bounded streaming/chunking that preserves vanilla capacity;
  no arbitrary entity-count cap or truncated data.
- **Benefit gate:** recommendation: require measured total tick improvement with
  the per-entry import and GC included before enabling a performance claim.
  NeoForge 26.3 equivalence remains unverified until public evidence is available.
