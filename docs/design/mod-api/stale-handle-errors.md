# Design: stale handle errors

Provisional answer to [arismoko/horizon#1644](https://github.com/arismoko/horizon/issues/1644): **typed stale errors for ordinary handle reads and writes**. This question is small enough for an issue comment proposing the answer, as Ari permits; this short page records that proposal for review, not implemented behavior.

## Proposed contract

Reuse the [decide, fire, apply](decide-fire-apply.md) outcome vocabulary and [Part instance state](part-instance-state.md) identity rules. Ordinary reads return owned values or a typed refusal; writes return their operation-specific success or typed refusal. Do not expose decision tokens or apply outcomes as a second transaction API. Replace covered Option/bool methods and their ID helpers together, updating catalog, generated adapters, callers and horizon-machine-sample without compatibility aliases. A valid domain absence remains an optional success value where the operation actually permits absence.

Resolve identity through the existing lifecycle holder and authoritative table: server-wide epoch plus id for entities, no dimension and no second generation. `Missing` means `Stale(reason)`; `Taken` at admission means `Busy(reason)`. `Found` still requires authoritative residency/visibility validation. Recommend temporary inaccessible live state as admission `Busy(Unavailable)` (reason spelling provisional), never Stale or fabricated defaults. Reads must not load, pin, swap parked state or retry. During apply, a newly Taken or invalidated prerequisite is `Stale(reason)`, following the accepted action note. Transfer preserves entity identity and resolves its current level. World close invalidates old handles permanently; reload needs fresh handles, as the Part note requires.

Use `InvalidInput(reason)` for malformed operation-owned input, retaining structured text-form reasons for action_bar. Preserve meaningful existing cancellation/refusal semantics rather than classifying every false as stale. `Canceled` and `ListenerFailed` remain action outcomes only where dispatch actually occurs. Reads take `&dyn Host`; writes take `&mut dyn ServerHost`; catalog side restrictions still govern both targets. Wrong-side writes must fail compilation.

Writes participate in the existing transaction and revalidate before mutation. In lifted passes, retirement and journal emission wait for root commit; abort publishes no packets or effects. Refuse saves during an open action savepoint. No mutable borrow crosses dispatch. Caught native unwinding and wasm traps follow ListenerFailed and rollback policy; process-aborting panics cannot be recovered.

## Evidence and route owners

Current owners below were inspected; they establish entry points, not complete migration or a verified transitive route audit. Single-player server, LAN and dedicated server must share authoritative resolution; remote client reads must obey replica visibility without gaining write authority.

| Route | Verified file:line owner |
| --- | --- |
| Entity position / level | `crates/horizon-mod-vocab/src/host/entities.rs:123,143`; `crates/horizon-client/src/game/modding/native_entity_refs.rs:675,735` |
| Entity clear_fire | `crates/horizon-mod-vocab/src/host/entities.rs:744`; `crates/horizon-client/src/game/modding/native_entity_state.rs:232` |
| Player name / action_bar | `crates/horizon-mod-vocab/src/host/players.rs:54,302`; `crates/horizon-client/src/game/modding/native_server.rs:1152,1196` |
| Level block reads | `crates/horizon-mod-vocab/src/host/levels.rs:190,211,217`; `crates/horizon-client/src/game/modding/native_level_refs.rs:433,518,530` |
| Lifecycle lookup | `crates/horizon-client/src/join_seq/residency/table.rs:24` defines Found, Taken, Missing |

Vanilla 26.3 inspection via `/workspace/horizon/tools/rea_vanilla.py method net.minecraft.world.entity.Entity isRemoved` shows removal is tested by a non-null removal reason. It does not define Rust retained-handle errors or verify all read/write routes. This proposal changes Horizon lifetime reporting, not vanilla gameplay. The issue asserts no NeoForge counterpart. The public [26.3 Entity Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/minecraft/world/entity/Entity.html) was inaccessible; NeoForge 26.3 lifetime behavior is unverified. Require public documentation before adopting any claimed counterpart; no NeoForge source or patches were consulted.

Generate API-owned Wire values and typed refusals for native and wasm from one definition. Unsupported capabilities must refuse registration/load with the type and capability named, before first access. Treat input handles, discriminants, positions and text as untrusted; validate with existing authoritative scope and Wire/text bounds, checked arithmetic and operation-specific ranges. Preserve vanilla capacity; introduce no arbitrary new numeric cap. No mod-side revalidation protocol. Unused paths add only O(1) holder validation, with zero listener scans, callback dispatch, encoding or per-holder allocations; measure this before runtime closure.

## Acceptance matrix

Proposed implementation tests on native and wasm, not results of this documentation change:

| Fixture | Expected result |
| --- | --- |
| Live entity at [1,2,3] | Position success [1,2,3] |
| Retain handle across despawn; clear_fire | Stale; zero mutations |
| Close/reopen same world, reuse numeric id | Old handle Stale; fresh handle succeeds |
| Taken at admission / during apply | Busy / Stale; zero mutation and no retry |
| Live identity temporarily inaccessible | Busy(Unavailable); no load or fabricated name/block |
| Entity transfers X to Y | Same epoch/id resolves Y |
| Valid loaded air block | Successful air value, not refusal |
| Malformed text | InvalidInput with text-form reason; zero packets |
| Listener writes then stale apply or caught panic/trap | Rollback; zero committed effects; next built-in action succeeds |
| Wrong-side write | Compile failure on both targets |
| No listeners | Zero dispatch/encoding/all-mod scans |

## Proposed implementation stages

File: none. First confirm the unavailable mapping with the world-visibility/residency owner and inventory every ordinary operation and semantic absence/refusal. Then generate the shared result representation and migrate those methods, callers and sample together. Finally exercise real retained handles, transaction failure recovery, native/wasm parity and unused-path cost. Adjacent residency and visibility work supplies validation; it is not evidence that these result surfaces already exist.

## Open questions

- **Temporary unavailability:** recommend Busy with a structured Unavailable reason, preserving admission versus apply semantics. The world-visibility owner must confirm suspended-player and unloaded-level classifications before runtime code; do not infer stale merely from failure to access storage.
- **Per-operation refusal mappings:** recommend keeping domain absence separate from lifetime failure and retaining structured text errors. Audit each covered method before freezing public signatures and Wire discriminants.
