# Design: entity render state

Provisional answer to [arismoko/horizon#897](https://github.com/arismoko/horizon/issues/897):
introduce one owner-thread entity preparation operation, after complete base and
specialized extraction and before model posing or geometry emission. All entity
routes call it. Extend the frame-owned store accepted in
[PR arismoko/horizon#2357](https://github.com/arismoko/horizon/pull/2357), rather than adding an
entity cache, callback bus or second lifetime system. This is a proposal, not an
implemented API. That page/store is absent at this checkout (`5166fe4588`);
its implementation is a prerequisite, not presumed landed.

## Vanilla and NeoForge 26.3

Verified using `/workspace/horizon`: `python3 tools/rea_vanilla.py method
net.minecraft.client.renderer.entity.EntityRenderer extractRenderState`.
Vanilla copies type, interpolated position, invisibility, age, dimensions,
name/leash data, fire, outline and packed light into state. The corresponding
`callers` query reports 25 references with complete coverage, including
`createRenderState`, living renderers and specialized boat, minecart, projectile,
display and other renderers. This establishes that the base extractor alone is
not the final specialized-state boundary. It does not establish a mod hook or
complete higher-level world/UI caller coverage.

The issue requests `RegisterRenderStateModifiersEvent`. The public
[26.3 Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/client/event/RegisterRenderStateModifiersEvent.html)
was inaccessible. Exact registration types, callback order, threading, field
coverage and limits are **unverified**. Obtain public documentation before
adopting the final counterpart contract. No NeoForge source, patches or other
project renderer source was read. The limits below are Horizon proposals.

## Verified routes and preparation boundary

Owners relative to the repository at `5166fe4588`; these are current integration
points, not existing extracted-state implementations.

| Route | File:line owner | Required treatment |
| --- | --- | --- |
| Mob library rendering | `crates/horizon-client/src/mobs/render.rs:1487`, `:1503` | Split extraction from render/render_with; consume prepared values only |
| Gameplay mob and remote-player geometry | `crates/horizon-client/src/game/mobs/creatures/mod.rs:8016`, `:8134`, `:8215` | Prepare before replacement hooks, body, layers and outlines; never fire separately per layer |
| Own-player geometry | `crates/horizon-client/src/game/interface/perspective.rs:786` | Use the same operation before player replacement and built-in posing |
| Existing replacement adapters | `crates/horizon-client/src/game/modding/native_mob_draw.rs:53`, `:71` | Read the prepared snapshot; do not redispatch modifiers |
| Frame preparation and consumer | `crates/horizon-client/src/game.rs:15048`, `:15720`, `:15040` | Collect prepared records into the existing frame packet before freeze and consumption |
| Frame owner | `crates/horizon-client-render/src/gfx.rs:1104` | Extend arismoko/horizon#2357 ownership and leases, not a parallel store |

First decide the view and candidate using existing visibility policy. Capture a
complete API-owned base plus supported specialized record; fire the matching
modifiers with ClientHost; validate and apply atomically; then pose and emit
commands exclusively from the resulting record. Do not modify a live Mob and
restore it afterward. Final dispatch belongs in gameplay, with library helpers
returning decisions/records and never reaching upward into Game.

World mobs, remote and own players must share this operation. Inventory/entity
previews, spawner displays, first-person arms, projectiles, item/display entities,
shadow views and screenshot/split-view entry paths require a full caller audit
before implementation: the table is verified evidence for the named paths,
not a claim those additional routes are covered. Recommend full-entity previews
use view-scoped records; first-person arms remain a separate presentation unless
public counterpart evidence requires them. Unsupported specialized schemas must
be refused at registration, never silently skipped after accepting registration.

## Proposed contract

`register_render_state_modifiers` is a client registration hook with ClientHost
and a typed registrar. Resolve entity types and supported state schemas once at
load into dense per-type subscriber lists. Dependency order applies identically
on native and wasm; within one mod use registration order. Register against a
supported state capability, not renderer implementation classes. Return named
`UnsupportedEntityRenderStateCapability` for unavailable fields or schemas,
`InvalidEntityRenderStateSchema` for invalid bounds, and an owned unknown-type
refusal only for this registrar's explicit type declarations.

Use API-owned EntityRenderStateWire records: fixed-width scalars, enums, vectors,
resolved resource keys and bounded custom fields using arismoko/horizon#2357's Wire schema.
No engine Mob, pointers, Any, GPU handles or mutable live references cross the
adapter. Start with presentation fields such as tint, scale, outline and supported
pose inputs. Identity, type and lifetime are immutable. A dependent listener sees
the previous listener's validated result. Existing replacement hooks remain
separate drawing moments and consume that same result without another modifier
call. DrawContext exposes immutable snapshot reads; workers never call mods.

Add entity records as a namespace inside arismoko/horizon#2357's frame-owned store, keyed by
view and the existing entity lifecycle holder. Never create a second entity
generation. Preview records use frame-local occurrence identity, not fabricated
persistent entity handles. Reuse frame/world/resource/target/mod epoch checks,
freeze, deferred-consumer leases, backpressure and teardown from arismoko/horizon#2357. Values
reset each frame. One entity in two views has two records; body, armor, outline
and shadow consumers in the same prepared view share one record. Reload or
lifetime change before apply refuses publication rather than recomputing.

Host writes and state apply follow decide/fire/apply in one existing transaction.
Inside lifted passes retirement and journal emission wait for root commit;
rollback publishes no record or journal. Saves are refused while an action
savepoint is open. Preparation must not leak a record from an uncommitted pass.
No additional transaction or identity mechanism belongs in this feature.

## Untrusted input, bounds and cost

Inherit arismoko/horizon#2357's slot, byte, nesting and in-flight ceilings, charged to the same
frame budget. Proposed additional custom data ceiling: 4 KiB/entity/mod and
256 custom fields, counted in the shared per-mod/frame budget. Do not cap the
number of vanilla entities or truncate vanilla extracted fields to meet a mod
budget: refuse the offending mod contribution and preserve the base record.
Check encoded and decoded lengths with checked arithmetic before allocation.

Validate modified floats as finite; proposed scale range [0,16], tint channels
[0,1], outline a u32 color, pose angles [-360,360] degrees. These restrictions
apply to writes, not untouched vanilla values. Resource keys must match captured
generations. Reject NaN, infinity, negative scale, excessive values and invalid
tags atomically, retaining the previous valid record. A failed listener cannot
undo earlier listeners. Native unwind, wasm trap or fuel exhaustion rolls back
its contribution and Host writes, disables that listener for the mod generation,
and emits one diagnostic. Native aborts cannot be recovered or callbacks safely
preempted. Wasm receives bounded fuel; neither target runs arbitrary destructors
on submission workers.

No listeners: one cached global enabled-bit branch and, when enabled elsewhere,
one dense per-type empty check, O(1). Zero modifier calls, extension allocations,
serialization or scans on an unmatched entity. Keep the existing baseline path
when globally unused. With listeners, dispatch only matching registrations;
reuse arenas after frame leases expire. These are acceptance requirements, not
measured performance claims.

## Acceptance matrix

Future implementation expectations, not results of runtime tests in this note.

| Case | Expected result |
| --- | --- |
| Pig modifier sets scale 2; cow base scale 1 | Pig draws at 2, cow at 1 |
| Own and remote player set tint red | Both draw red, including their armor/outline consumers where supported |
| Two dependent modifiers set scale 2 then multiply by 3 | Final scale 6; each called once per entity/view |
| Body plus 3 layers in one view | 1 modifier dispatch, 4 consumers of the same record |
| Same entity in two prepared views | 2 dispatches, isolated records |
| Frame N writes red, N+1 omits tint | N+1 uses base tint |
| NaN/infinity, scale -1 or 17, 4097 custom bytes | 0 offending changes; base/earlier valid state survives |
| Native unwind or wasm trap after Host write | 0 committed writes, 1 diagnostic, listener disabled |
| Entity retired/reused or resource epoch changes before apply | 0 stale publications |
| Lifted pass rolls back after preparation | 0 published records and journal entries |
| Save requested during open action savepoint | Save refused |
| Vanilla scene exceeds mod byte budget | All vanilla entities retained; offending mod contribution refused |
| Unsupported specialized schema | Registration returns UnsupportedEntityRenderStateCapability |
| Native and wasm receive identical Wire input | Identical state values and refusal results |
| No listeners across 100 frames | Baseline geometry identical; 0 modifier calls/allocations/scans |

## Proposed stages (file none)

1. Approve this boundary and verify public counterpart semantics; finish the vanilla and Horizon world/UI caller census.
2. Implement arismoko/horizon#2357's shared frame store and snapshot-only draw boundary; add entity records within it and split all accepted routes into extraction and consumption.
3. Add the registrar, catalog and generated native/wasm adapters together, then demonstrate a type-specific visible modification and run the matrix plus unused-path allocation measurements.

## Open questions

- Exact NeoForge 26.3 contract is unverified. Recommend public Javadoc evidence before freezing signatures or claiming equivalence.
- Which specialized schemas and preview routes ship first? Recommend common/living/player schemas together, explicit refusal for others, and full-entity previews included; finish the route audit before accepting registrations.
- Are proposed custom-data budgets sufficient? Recommend the shared frame ceilings plus 4 KiB/entity/mod initially; measure real scenes without reducing vanilla capacity.
