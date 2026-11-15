# Design: moving structures

Proposed contract for horizon-rust/horizon issue arismoko/horizon#794; this document requests design
agreement and does not claim an implemented feature. One game-owned carrier entity
owns a sparse collection of local block cells, their block entities and attachments.
Mods control it through typed handles and Parts, without depending on game internals.

## Proposal reviewed and representation chosen

[horizon-rust/horizon#5313](https://github.com/horizon-rust/horizon/issues/5313)
proposes ownership, collision, persistence, synchronization and rendering, and leaves
carrier identity, residency and rotation undecided. Its maintainer comment places the
work on the Later roadmap and explicitly calls for several pull requests.
Adopt its entity direction and reuse of transformed block drawing. Reject a controller
block entity as the authoritative owner: crossing chunks must not split ownership.
Reject a disguised vanilla entity type; reserve a game-owned protocol type with a
negotiated feature requirement. General mod entity registration is not a prerequisite.
Reject piston redirection as a prerequisite: it changes a different operation and
cannot express rigid rotation. Include rotation in the initial simulation contract;
do not ship translation semantics that later require replacement.

The proposal's existing-source locations are historical. Verified current foundations
are `crates/horizon-world/src/level/pistons.rs:22` (single-block MovingPiston),
`crates/horizon-client-render/src/collisions.rs:85` (entity AABB collision channel),
`crates/horizon-client/src/game/entities/minecarts.rs:2679` (transformed block models)
and `crates/horizon-client/src/game/entities/text_displays.rs:299` (their display use).
These are integration foundations, not an existing moving-structure implementation.

Vanilla 26.3 evidence: running `python3 tools/rea_vanilla.py method
net.minecraft.world.level.block.piston.PistonMovingBlockEntity tick` from
`/workspace/horizon` shows progress advancing by 0.5, collided/stuck entity movement,
and final replacement with neighbour updates; clients retain a completed piston for
up to five death ticks. Retain ordinary stationary block semantics; do not generalize
that short piston animation into structure lifetime. No vanilla carrier counterpart
exists. No NeoForge or mod implementation source was consulted.

## Ownership and transactions

A level-owned carrier has a UUID, generation, dimension, transform, revision, cells
and local scheduled work. Cells are integer local positions with block state, optional
block-entity payload and attachments. There is exactly one ticker and one inventory
owner per cell. Assembly transfers ownership; it never clones live block entities.
Its world side, taking a block entity out with its attachments and none of its removal
side effects and installing it elsewhere as a new holder, is `BlockEntities::carry` and
`land` (`crates/horizon-client-render/src/block_entities/carry.rs`), which pistons use
for a block whose part answers `piston_carries_block_entity` with `Some(true)` (arismoko/horizon#3184).

Assembly preflights a bounded selection in loaded, ticking chunks, permissions,
block support and cell epochs. A server callback may veto within a transaction;
revalidate the exact selection, epochs and residency after callback writes. A mismatch
refuses without partial transfer. Successful commit removes world tickers, transfers
attachments and scheduled work, creates the carrier and emits ordinary world removal
notifications once. Reject external pending piston moves or unsupported block entities.
No drops are produced by transfer.

Disassembly requires a grid-aligned translation and a cube-symmetry rotation, an
explicit block-state rotation adapter, loaded writable destinations and no occupied
cell. Preflight all cells before mutation. Refusal preserves the carrier. Commit
transfers each payload once, installs world tickers and then runs neighbour updates.
Unsupported rotations never silently rewrite facing properties. Deletion is a separate
explicit transaction: dismantle into storage or authorized destruction with drops;
entity death must never implicitly duplicate or discard carried inventories.

## Transform and contact contract

For local point p, pivot c, translation t and normalized quaternion q, world position
is `t + c + R(q)(p - c)`. Store f64 translation and canonical normalized rotation;
block states stay in their local orientation while carried. World queries and rendering
transform shapes, normals and interaction rays rather than rewriting states each tick.

Broad phase uses the union of transformed cell bounds. Narrow phase uses the actual
block voxel boxes transformed to oriented boxes. Use swept separating-axis tests for
translation, and conservative advancement with bounded angular intervals for rotation;
an unresolved interval stops motion conservatively. Never substitute the carrier's
outer AABB as a solid shape: gaps must remain traversable. Stable UUID/cell ordering
breaks simultaneous-contact ties. Motion commits only a collision-safe fraction;
contact resolution and carried entity displacement commit together. No tunnelling,
crushing through static walls or client-supplied contact authority is permitted.
Standing players and mobs receive the contact point's rigid displacement, including
rotation; jumping releases support. Explicit passengers use entity riding rules,
separately from standing contacts. Projectiles and ray targeting use the same local
shape index; explosions and melee address local cells through a validated hit handle.
Pathfinding may conservatively avoid moving geometry until navigation support lands.

## Route and lifecycle coverage

The common server-level owner is currently
`crates/horizon-client/src/game/world/levels.rs:652` (`tick_server`); guest action scope
is `crates/horizon-client/src/game/network/lan_actions/turns.rs:6` (`as_guest`).
Integrate the proposed carrier manager there so integrated, LAN and dedicated modes
share simulation. The following are required routes for the staged implementation,
not claims that the existing piston/display paths already support carriers.

| Route | Proposed owner and outcome |
| --- | --- |
| Assembly, command or mod request | Server carrier manager executes the same atomic transfer. |
| Translation, rotation, standing, pushing, riding | Server contact solver advances once per level tick; clients interpolate. |
| Local block-entity tick and scheduled tick | Carrier local level adapter ticks once; adjacent cells resolve locally. Unsupported world-coordinate-dependent tickers refuse assembly until adapted. |
| Use, mining, melee, projectile, explosion, automation | Server validates UUID/generation/local cell, reach and revision; local adapters preserve ordinary permissions and transactional Parts. World queries outside the structure use transformed positions explicitly. |
| Chunk crossing | Level manager retains ownership; acquire bounded swept-volume tickets before motion. Missing tickets pause motion and local ticking, without partial advancement. |
| Dimension crossing | Explicit atomic transfer after destination residency and collision preflight; otherwise refuse. Ordinary movement never changes dimension. |
| Unload, shutdown, restart | Persist the complete carrier before releasing tickets; dormant carriers do not tick. Resume only after full residency. |
| Disconnect or controller removal | Carrier survives independently; controller references are weak handles. No implicit disassembly. |
| Save of a parked dimension | Same carrier serializer, integrated with the parked-level save route at `crates/horizon-client/src/game/world/levels.rs:1040`. |
| Late join, tracking entry, tracking exit | Snapshot before deltas; exit disposes client state only. |
| Resource reload, render disposal | Rebuild model caches using new resource generation; release meshes on tracking exit/disconnect. |
| Occupied or unloaded disassembly target | Refuse atomically; ownership and counter remain in carrier. |

A local adapter must explicitly opt block types into supported ticking, scheduled work,
menus, capabilities and neighbour semantics. Redstone stays local; cross-boundary
connections require an explicit bridge Part. Do not make arbitrary vanilla tickers
believe local coordinates are stationary world coordinates.

## Save, synchronization and rendering

Persist versioned cells, canonical registry names, block-entity data, attachments,
local schedules, UUID, transform and simulation revision in a level carrier store.
Chunk references are indices, never duplicate payload owners. Journal assembly and
removal together with affected chunks so crash recovery replays a single committed
ownership transaction. Unknown required content quarantines the complete carrier
without ticking or deleting its data. Unsupported schema fails load explicitly.

Tracking uses transformed bounds and dimension. Send a bounded manifest and segmented
snapshot with checksum, then revision-ordered transform and cell deltas. Clients expose
nothing until the complete snapshot validates. Gaps request a fresh snapshot; older
generations cannot resurrect a removed carrier. Server checks interaction packets
against authoritative geometry. Negotiate support before joining a world with carriers.

Interpolate translation and shortest-arc quaternion rotation between server samples;
teleports reset interpolation. Cache local block meshes by cell/resource revision,
reuse transformed model drawing and add local block-entity renderer contexts. Transform
lighting sample points to world space; translucent sorting uses camera world distance.
No client ticker may change authoritative inventory or counters. Typed DrawContext
services expose rendering to mods without exposing Game or GPU ownership.

## Proposed mod_api surface and trust boundary

These names are proposed additions, not available APIs. `LevelRef::assemble` returns
`StructureRef` (UUID plus generation). `StructureRef::motion`, `cells`, `disassemble`
and attachments have natural handle receivers; reads take Host and writes ServerHost.
`StructureCellRef` includes carrier generation and cell epoch. Scoped
`StructureMotionPart` supplies a desired rigid step; `StructureCellPart` declares
transfer, local tick and state-rotation support. Existing item/fluid/energy Parts are
reached through the carried cell's holder under the same transaction. Register once,
resolve dispatch at load and expose the same generated native/wasm boundary.
Each lifecycle callback receives ServerHost; rendering receives ClientHost and
DrawContext. Decide, fire, revalidate, apply; no parallel compatibility hook family.

Treat all mod values, persisted data and network payloads as untrusted. Initial
limits: 4,096 non-air cells, local coordinates -128..127, 16 MiB decoded carrier data,
64 KiB per cell, 64 resident chunks per carrier, translation at most 1 block/tick,
rotation at most 5 degrees/tick, and finite world positions inside the world border.
Reject duplicate cells, arithmetic overflow, invalid registry values, NaN/infinity,
zero quaternions and over-budget snapshots before allocation or mutation. Normalize
finite nonzero quaternions; refuse excess motion rather than silently clamping it.
Budget transformed shape boxes and collision iterations as well as cell counts;
exhaustion refuses the step. Revalidate bounds after callbacks. Catch unwind at the
mod boundary, roll back staged writes and disable the failing callback for the session;
wasm traps follow the same policy. Native abort cannot be recovered by this contract.

Tick cost scales with resident carriers, active local tickers and nearby candidate
contacts; cache shape indices and dirty geometry. With no carriers there is no world
cell scan or per-mod dispatch. Never allocate per cell merely to interpolate a frame.

## Concrete acceptance scene

Use two unit solid cells at (0,0,0) and (1,0,0), pivot (0,0,0), translation
(10,64,10), identity rotation. Origins are (10,64,10) and (11,64,10).
Positive 90-degree Y maps +X to -Z: after eighteen accepted 5-degree steps, origins
are (10,64,10) and (10,64,9). One carried counter starts at 40: one active tick yields
41, with zero world ticker remaining; a paused tick leaves 41. Save/restart preserves
both cells, transform and counter. Late join matches the existing client's revision.

Translate the identity carrier toward a wall at x=13 by one block per tick. Its initial
maximum x is 12; the first step reaches 13, the next is blocked with zero penetration.
A supported actor at (10.5,65,10.5) moves to (11.5,65,10.5) on that first step.
In a separate three-cell-span fixture containing only x=0 and x=2 cells, a ray down
through local (1.5,2,0.5) misses; an outer-AABB collision implementation fails this case.
An obstacle intersecting only the intermediate angular sweep must stop rotation even
when both endpoint poses are clear.

Occupied disassembly leaves two carrier cells and zero destination writes. Clear the
targets and disassemble at a grid pose: two world cells, one world counter ticker,
zero carrier cells. Replay the old handle: stale-generation refusal and no writes.
Repeat with ticket refusal, disconnect, parked dimension and crash recovery. Run the
same scene on integrated, LAN and dedicated servers with native and wasm callers;
invalid motion followed by a valid built-in step must leave normal simulation working.
These are required future acceptance tests, not results of this documentation change.

## Proposed board issues

File none during this design task. Each slice can land disabled until its dependencies
exist; each includes its own refusal and lifecycle tests.

1. Carrier data model and bounded codec: local cells, UUID/generation and transforms;
   test exact transform values and malformed input without gameplay integration.
2. Transactional ownership store: atomic assembly/disassembly, journal recovery and
   parked-level persistence; depends on 1, tests crash replay and single inventory owner.
3. Residency and scheduling: ticket budgets, pause/resume and dimension transfer;
   depends on 2, tests no ticks during incomplete residency.
4. Local block adapters: opt-in ticking, schedules, attachments and transactional
   capabilities; depends on 2, tests counter 40 to 41 and unsupported-type refusal.
5. Rigid contact kernel: swept translation/rotation and sparse shape queries, usable
   independently in a geometry harness; tests gaps, angular sweep and iteration limits.
6. Server contact integration: standing, pushing, passengers, ray/projectile/explosion
   routes and validated interactions; depends on 3–5, tests common server modes.
7. Tracking protocol: feature negotiation, bounded snapshots, deltas and reconnect;
   depends on 1–3, tests reordered packets and late join without requiring rendering.
8. Client drawing: cached block and block-entity meshes, interpolation, reload disposal;
   depends on 7, tests transformed output and resource invalidation.
9. Public handles and Parts: generated native/wasm surface and callback rollback;
   depends on 2–6, tests stale handles, invalid output and panic/trap recovery.
10. End-to-end enablement: acceptance scene across server modes, both mod targets,
    measured contact/tick budgets and user documentation; depends on 6–9.

## Open questions requiring agreement

- Which vanilla block entities enter the first support set? Recommend explicit adapters
  for passive inventories plus a simple ticking counter before world-dependent machines;
  unsupported types refuse assembly. A per-type audit belongs to issue 4 above.
- What aggregate server budget should apply across many carriers? Recommend a global
  residency and contact-work quota in addition to the per-carrier limits above; choose
  default totals from the acceptance workload measurements before enablement.
- Is crash-atomic cross-region transfer acceptable as a prerequisite? Recommend yes:
  ordinary independent chunk saves cannot guarantee single ownership after power loss.
  Agree the journal storage format in issue 2 before exposing assembly publicly.
