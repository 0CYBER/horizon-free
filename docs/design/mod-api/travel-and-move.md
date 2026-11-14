# Design: travel and move hooks

Proposal for [#1009](https://github.com/arismoko/horizon/issues/1009), not an implemented API.
This is a clean-room Horizon design from vanilla 26.3 and the requested use cases.
There is no NeoForge counterpart. No other project's source informs this proposal.
Code follows agreement on this note.

## Moments and typed contract

Declare exactly two catalog moments: `living_travel` and `entity_move`. Travel
uses `HostSide` with typed `ServerHost` or `ClientHost` access, never an
unchecked side flag granting server writes. Entity reads and authoritative
writes belong on typed entity/player handles; do not add entity-id movement
methods to Host. `entity_move` is ruled otherwise (#2414 maintainer ruling): a
loop-context hook **without a host**, like `ender_entity_teleport` — there is
nothing at that point in the reference for the hook to mirror, so the listener
only decides the delta: continue, cancel or replace.

Travel fires once at the LivingEntity.travel boundary, after ordinary input and
attribute resolution and player swimming preparation, before fluid/air/glide
selection. Its owned snapshot contains a living-entity handle, optional player
handle, level/entity epoch, input vector, velocity, orientation, grounded/fluid/
flying/gliding state, resolved attributes, simulation tick and action identity.
The output is `Continue`, `Cancel`, or `Replace { input, velocity }`. Replacement
supplies the branch's starting values; it does not replace the physics algorithm.
Cancel skips this travel invocation, including its nested moves and flying velocity
restoration, leaving its entry position and velocity intact. Earlier input/jump
processing is not undone. Speed, jump, gravity and step remain attributes; neither
hook adds multipliers or a parallel attribute system.

Move fires once per physical Entity.move invocation, including external pushes,
before no-physics handling, piston limiting, stuck multipliers, edge backoff and
collision. The listener hears the entity, its kind, the requested delta and the
typed cause (Self, Player, Piston, ShulkerBox or Shulker — vanilla's
five `MoverType` names; boats and minecarts move as Self, `Player` is the
server packet sites). Output is
`Continue`, `Cancel`, or `ReplaceDelta(Vec3)`. Cancel skips that invocation without
changing position, velocity, collision flags or fall state and emits no displacement
notice. Replacement still passes through all ordinary restrictions. Collision,
step height, grounding, fall damage, restitution and movement emissions use the
actual resulting displacement in vanilla order. Travel and move are distinct
moments: a travel replacement can still encounter a move veto. No second hook is
added for individual branches or replay.

Vectors use owned finite f64 components. Travel input components must lie in [-1,1];
replacement velocity and move delta components in [-64,64] blocks per tick or call.
These are proposed mod-output budgets, not limits applied to vanilla Continue.
Reject the entire malformed result (NaN, infinity, out-of-range, including negative
magnitude budgets); signed negative vector components within range are valid.
Reject rather than clamp, retain the preceding valid proposal, and diagnose once
per listener/tick. Dependency-ordered listeners see the preceding valid proposal;
Cancel is terminal. For `entity_move` — with no host writes there is nothing to
go stale — a panic or wasm trap keeps the **requested** delta (not a replacement
an earlier listener had proposed), and the panic poison disables that listener
for the session; the move carries on. A rejected replacement is refused as the
requested delta would be: result validation, not a stale-decision recheck.
Native abort cannot be recovered. Native and wasm otherwise share validation
and see the same hook through the generator.

## Ownership and route map

References verified at worktree HEAD `9556bd72a8`; paths below are relative to
`crates/horizon-client/src/`. These are current owners, not claims that hooks exist.

| Route / owner | Current file:line | Required integration |
| --- | --- | --- |
| Player travel / air / glide | `motion.rs:1487`, `:1167`, `:2187` | One travel decision before branch selection; water, lava, air, glide and creative flying covered |
| Physical player movement | `motion.rs:2297`, trait adapter `:3039` | Shared move operation, retain cause through adapters |
| Shulker box push / piston limit | `motion.rs:2292`, `:2492` | Same move moment before external-motion restrictions |
| Controlled window simulation | `game/network/remote/mod.rs:7129` | Predict with client-side context |
| Local tick / replay | `motion.rs:1367`, `lan/prediction.rs:900` | Same deterministic decision/apply path; replay is not a new action |
| Integrated, LAN and dedicated guest turns | `game/network/lan_actions/turns.rs:6`, `:18`, `:45` | Server context bound to acting guest, never host player identity |
| Action service and scoped fake actors | `game/modding/native_server.rs:538`, `:549` | Enter existing player action validation; physical motion subsequently uses shared hooks |
| Normal living mobs | `mobs/living.rs:1451`, `:1544` | Lift decision to game adapter; include mob air and fluid/glide equivalents, not just player motion |
| Shared air equations | `../horizon-world/src/living_air.rs:134` | Keep physics below game independent of Host |

The last path is relative to the crates directory instead of the client source
root: `crates/horizon-world/src/living_air.rs:134` is the shared owner.
Vehicle movement must use the entity move boundary for the vehicle itself;
passenger positioning is not a second travel or physical move. Shulker pushes and
other external physical displacement preserve their cause and invoke move even
without travel. Teleports, packet corrections, remote interpolation and passenger
seat assignment are position assignments, excluded from both hooks. Remote entities
are not predicted locally; their authoritative server invokes the hooks. An action
service request is not itself travel: do not manufacture movement on attack/use.
Implementation must inventory vehicle and nonliving collision adapters before code
lands and route their actual Entity.move equivalents through this boundary.

## Transaction, prediction and replay

`living_travel` uses decide/fire/apply: lower physics prepares a bounded
immutable decision, the game dispatches with Host, and apply revalidates entity
existence, level epoch, position, velocity, attributes and collision/environment
revision in the same transaction as listener writes. A changed precondition
refuses the decision and rolls back that transaction; never recompute against
changed state mid-callback. `entity_move`, ruled hostless (#2414), reduces to
result validation: the armed gate holds the requested delta, a listener result
that passes the budget replaces it, malformed output falls back to it, and
cancellation skips the invocation. A panic keeps the requested delta while its
poison disables that listener for this hook for the session. Both take
the existing path without snapshots, allocations or mod scans when nobody
subscribes — dispatch only to subscribed mods, resolved once.

Integrate with the [client action pipeline](client-action-pipeline.md), preserving
its input/action ordering and server validation. Negotiate movement-rule identity
and deterministic configuration before enabling prediction. Server validates intent,
attachment state, reach and environment independently; it never trusts a client's
replacement delta. Server simulation is the sole authoritative result. Unmatched
client rules disable custom prediction and await authoritative movement.

Record input, tick/action identity, mod rule version and deterministic attachment
state with each prediction checkpoint. Corrections restore those values and rerun
the same rules against corrected environment snapshots. Do not reuse outputs from
a divergent environment. Predicted callbacks may update only rollback-owned state;
server writes remain server-only. Sound, particles, packets, damage and other
external effects commit through an identity-deduplicated effect journal, never
inside replay callbacks. Server damage uses the authoritative collision result.

A grapple stores an anchor on the entity and contributes bounded velocity before
travel; server checks anchor reach and obstruction. A glider replaces travel input
and starting velocity while vanilla chooses the glide branch and applies gravity.
Wall-running uses a recorded wall normal to remove velocity into the wall, then
ordinary collision prevents penetration. Loss of anchor/wall invalidates the rule;
client and server return Continue from the same rule rather than inventing a
teleport. Each example uses attributes for speed/jump/gravity/step adjustments.

## Acceptance matrix

These are implementation requirements, not tests claimed to have run. Geometric
cases isolate move from travel acceleration and gravity.

| Fixture | Concrete expected result |
| --- | --- |
| Empty world, start (0,64,0), delta (1,0,0), Continue | Position (1,64,0), one committed displacement notice |
| Same, Cancel | Position (0,64,0), zero displacement notices |
| Same, ReplaceDelta (0,0,1) | Position (0,64,1), one notice |
| AABB x extent [-0.3,0.3], solid wall plane x=1, delta (2,0,0), no step opening | Center x=0.7, right face x=1, zero penetration |
| Replacement NaN, infinity, 65 or -65 on one axis | Entire result rejected; original delta (1,0,0) still yields (1,64,0) |
| Replacement (-1,0,0) | Position (-1,64,0) |
| Listener panics after attachment write | Write rolled back; original delta applied; listener disabled |
| Callback changes level epoch or starting position | Stale decision refused; zero movement from that decision |
| Two identical corrected replays of fixed input sequence | Final position/velocity and attachment bytes equal fresh authoritative simulation; exactly one external effect per action identity |
| Host window, LAN guest, dedicated player; native and wasm | Each geometric case above has identical values and correct acting-player identity |
| No listeners: air, water, lava, glide, flying, piston, vehicle | Exact position/velocity/collision/fall-state equality with vanilla 26.3 oracle fixtures for the same inputs |
| Teleport/correction/interpolation/seat assignment | Zero travel/move hook calls for the assignment itself |

Vanilla was inspected with `python3 tools/rea_vanilla.py method
net.minecraft.world.entity.LivingEntity travel` and the corresponding `Entity move`
query using the prepared jars under `/workspace/horizon`. The former selects fluid,
glide then air; the latter limits piston movement before collision and applies fall
and emission behavior afterward. No vanilla implementation is copied here.

## Open questions

- Exact vehicle/nonliving adapter census is implementation work still needed.
  Recommend requiring a file:line route inventory and oracle fixtures before the
  hook implementation is approved; never ship player-only coverage under Entity.move.
- Prediction checkpoint transport for mod attachment state is not specified by the
  existing action note. Recommend a negotiated, versioned bounded schema (64 KiB per
  entity checkpoint, reject overflow without prediction), with server snapshots and
  rule identity; agree its wire ownership before implementing prediction support.
