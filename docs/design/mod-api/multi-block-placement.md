# Design: multi-block placement

Proposed contract for [arismoko/horizon#855](https://github.com/arismoko/horizon/issues/855), not an implemented feature. This replaces the fixed placement preview. The decisions below require review before implementation.

## Evidence and limits

Vanilla 26.3 was inspected with `python3 tools/rea_vanilla.py method <class> <method>` from `/workspace/horizon`: `net.minecraft.world.item.BlockItem place`, `net.minecraft.world.level.block.piston.PistonBaseBlock onPlace`, and `net.minecraft.world.level.block.TripWireBlock onPlace`. BlockItem writes first, applies item block state and block-entity data, calls setPlacedBy, triggers placement criteria, emits sound and a game event, then consumes one item. Piston onPlace calls checkIfExtend on the server when replacing another block and no block entity exists; tripwire onPlace calls updateSource when replacing another block. These synchronous calls are the reason a fixed partner list is insufficient.

The permitted [26.3 EntityMultiPlaceEvent Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/event/level/BlockEvent.EntityMultiPlaceEvent.html) was requested, including its extensionless URL, but returned `usage_exceeded`; the web reader also failed. Consequently its exact capture boundary, notification ordering, cancellation domains and snapshot representation are **unverified**. No claim of exact NeoForge equivalence is made. The issue identifies the counterpart; its contributor's mechanism description is deliberately not evidence for this design. No NeoForge source or patches were read. Public contract verification is an implementation prerequisite, not permission to infer undocumented internals.

## Capture boundary and moment

Use full synchronous execution journaling, preserving vanilla ordering. Start an action savepoint before placement resolution (including random draws); execute real placement against reversible state through Sim. Include all successful block mutations transitively caused before the placement call returns, including setPlacedBy, onPlace, synchronous neighbour notifications, fence connection shape changes and tripwire source changes. Record positions in first-write order, with original state and final state; retain the ordered write journal internally, including repeated writes and positions restored to their original state. The payload contains each touched position once, even if its final state equals its original state. Listener writes are not added to this frozen payload.

A queued piston block event belongs to the action's effects, but a piston movement executed in a later event-processing phase does not belong to its write set. Scheduled ticks and independently processed neighbour work likewise remain outside the set even if placement scheduled them. Do not drain global queues to enlarge capture. This causal call boundary, rather than block class or elapsed tick, also defines tripwire inclusion.

Fire server-side entity_place after synchronous execution, before publishing or committing it. Hook docs must state that listeners read tentative block states, but tentative entity changes are not visible in entity rows until commit. Single is one touched position; Multi is two or more, delivered through existing Placement::Multi. Primary pos/state and placed_against preserve their existing meaning; capture placed_against before writing. Failed placement emits no entity_place.

This deliberately changes the existing “before it is written” promise in `crates/horizon-mod-vocab/src/hooks/entity_place.rs:4` and `hooks/notes.rs:2859`: update both, generated documentation and samples together. It is a veto after tentative writes, not a pre-write preview. A separate pre-action permission hook and a committed notification are different moments if required; never alias either to entity_place or fire it twice. Any existing committed notice remains after successful root commit. No per-class hooks, revived preview or compatibility path.

## Cancellation domains

| Domain | Required cancellation behavior |
| --- | --- |
| Block writes and direct neighbour updates | Restore in reverse mutation order, silently; do not invoke removal/onPlace/neighbour callbacks during undo. Undo every synchronous derived write. |
| Item use | Restore full stack/components and count, equipment and player action bookkeeping. Survival success consumes one; creative success consumes zero. |
| Queued effects | Discard action-local sounds, particles, criteria, game/level events and drops; publish only after root commit, in original order. |
| Piston block events | Remove only newly enqueued events; restore deduplication metadata without deleting pre-existing events. |
| Scheduled ticks | Restore queue contents, ordering, identities and deduplication state, including replaced entries. |
| Level random | Restore complete generator state and draw position; listener draws also roll back on veto. |
| Block entities | Restore removed instances and mutable contents; remove created instances and invalidate their handles. Attachments key on (position, generation); defer retirement of replaced block-entity attachments until root commit. Restore identity/generation safely; no leaked tentative handle may retarget another instance. |
| LAN/client sends | Hold block logs, light/remesh invalidations and outbound effects until commit. Cancellation sends no placement changes; protocol-required acknowledgement/correction of client prediction is allowed and reflects original blocks and inventory. |

The savepoint must also cover synchronous entity creation/removal and inventory drops. Following the [residency and visibility design (arismoko/horizon#1606)](https://github.com/arismoko/horizon/issues/1606), defer attachment-holder retirement and journal Remove emission until root commit, as with effects. Rollback discards tentative spawns without retiring anything installed only tentatively; preserve restored entities’ attachments and join numbers. Use the existing residency boundary; no second raw path taking the entity list. Queue length truncation alone is insufficient when execution mutates existing entries. External irreversible I/O is prohibited during capture. This is a stronger explicit Horizon proposal where the unavailable public contract supplies no verified answer.

## Host transactions and deferred Sim apply

The current storage transaction is not a Sim journal (`crates/horizon-client/src/game/runtime/transactions.rs:192,386,403`). Extend its root coordinator, rather than introducing an independent transaction API. Begin an action child savepoint under any enclosing host.transaction. Perform capture only when the deferred action reaches its ordered root-apply position, with earlier stored writes visible. Preserve the decision's target, stack, dimension and relevant read-set versions; validate before execution and again after listeners. A changed precondition refuses this action; never recompute a replacement placement.

Freeze capture, then invoke listeners in a nested storage savepoint over tentative Sim state. A listener may write another block or consume a stack through Host; reads see its pending writes. A committed nested transaction merges into this action, never escapes it. On cancel/panic/stale refusal: first discard listener pending writes and deferred effects (and undo any listener eager storage changes), then undo already-applied listener Sim writes if any, then reverse placement Sim mutations, restore item/actor and RNG snapshots, and discard the action effect buffer. Finally close the action savepoint aborted. Never apply listener deferred block writes while unwinding. Thus a listener setting C=stone and canceling leaves C at its pre-action state even when its nested transaction committed. An outer abort also undoes an accepted action. Successful listener writes apply once in recorded order after validation, under the same journal; no second placement execution. Only successful root completion flushes effects and sends.

While an action savepoint is open, refuse every save, including listener `save-all`, with vanilla’s save-failed message. The visibility lease gate ([arismoko/horizon#2211](https://github.com/arismoko/horizon/pull/2211) and its follow-up) must count the open savepoint as a lifted pass; arismoko/horizon#2211 supplies the lease foundation, not the save gate itself.

## Owning routes to migrate

Paths below are relative to the repository at this design's base; these are inspected owners, not new APIs.

| Route | Owner and required treatment |
| --- | --- |
| World placement | `crates/horizon-world/src/level/place.rs:1791` enters Sim place at line 1686; writes at 1702/1704 and companion work at 1550. Central journal covers these and all transitive writes; instrument the common write boundary, not each block class. |
| Player/server guest | `crates/horizon-client/src/game/network/lan_actions/interaction.rs:241` owns authoritative guest UseOn. Single player also uses the server connection, as CLAUDE.md specifies. Route successful block-item use into the same action coordinator. |
| Mod/fake player | `crates/horizon-client/src/game/modding/native_server.rs:658` dispatches PlayerAction::UseOn to guest_use_item_on; same actor, inventory and refusal semantics. Raw mod block setters remain independent writes, not invented entity placements. |
| Mob | `crates/horizon-client/src/game/mobs/creatures/mod.rs:3237` selects enderman placement, line 3240 other mob placement; `native_hooks/shared.rs:2562` owns mob_place_block. Capture actual execution and preserve carried-block rollback; retain the separate enderman permission moment. |
| Dispenser | `crates/horizon-client/src/game/blocks/dispenser.rs:1656` calls place_block with a directional context; include its subsequent item state/block-entity work in the action. Use placer=None and restore dispenser inventory on veto. Other dispense actions that do not place blocks are excluded. |
| Remote prediction | `crates/horizon-client/src/game/network/remote/mod.rs:9605` executes place_block against DryLevel. It must not dispatch this server hook; authoritative correction resolves prediction. |
| Existing hook adapter | `crates/horizon-client/src/game/native_hooks/shared.rs:2508,2538` owns place_canceled/player_place_canceled and the fixed preview. Replace these callers and delete the preview transport and companion inference. |

Implementation must trace each authoritative item's use dispatch into this owner and test local, LAN, dedicated and fake-player routes. Silverfish infestation, turtle eggs and falling-block landing are currently excluded by hook notes; do not silently advertise them as covered entity placements. Adding them requires a separately verified placement contract, not a per-class preview.

## Trust, panic and cost

Payload is immutable, callback-scoped and cannot be retained as live handles. Validate IDs, dimensions, positions, stack limits and transaction capabilities at Host boundaries; reject unknown states owned by this operation, non-finite coordinates, invalid counts and stale handles before a mutation. Set cancellation monotonically across listeners. Bound nesting and journal growth with a declared budget; exceeding it refuses and rolls back the action, never truncates Multi. Catch native unwinding at dispatch and treat guest traps identically: abort the whole action, report/disable the failing listener under runtime policy. A process-aborting native panic cannot be recovered by a journal.

Resolve subscriptions at load. With no entity_place subscriber, bypass capture/payload allocation and execute the normal placement route; ordinary enclosing transaction guarantees still apply. No per-tick scan, allocation or all-mod dispatch. A journal-enabled transaction must not accidentally activate the hook. Measure subscribed cost proportional to actual mutations/effects and no-listener allocation count.

Mob, dispenser and fake-player routes must guard listener unwinding: restore the lift and close the action savepoint aborted. Document panic during restoration as a limit.

## Acceptance matrix for implementation

These are required expected values, not tests claimed to pass. Use fixed coordinates and explicit state properties, not implementation-generated expectations.

| Fixture | Expected result |
| --- | --- |
| Bed/door/tall plant at A/B, both initially air, stack 3 | Cancel: A/B air, count 3, zero published placement effects. Accept survival: both specified halves, count 2; creative count 3. |
| Fence A next to fence B with B east=false | Accepted Multi contains A and B; B east=true. Cancel restores B east=false and A air. |
| Powered piston placed at A facing east | Capture includes A; extension event is buffered. Cancel: A air, stack 3, zero new piston events. Accept: stack 2; processing the later extension event changes blocks outside this capture. |
| Two hooks at (0,64,0)/(4,64,0), facing inward; wires x=1,3; place wire x=2 | Capture includes x=2 and synchronous changed hooks/wires; after attachment hooks attached=true and wires attached=true, powered=false with no entity touching. Cancel restores missing centre and original detached states, stack 3; zero new published sounds/direct-update effects. Pin exact ordered touched-position list with vanilla before merging runtime support. |
| Cancel action scheduling ticks, drawing random and replacing a block entity | Serialized tick/event queues and RNG exactly equal pre-action snapshots; original block entity identity/data restored; created handles invalid; zero placement network sends and count 3. |
| Listener writes C=stone then cancels | C and placement positions equal pre-action states, count 3; nested listener commit cannot escape. |
| Listener nested transaction commits, then a later listener cancels; spawn inside the first listener | Placement and listener writes roll back; tentative spawn disappears, existing attachments and join numbers survive, zero published effects. |
| Save while action savepoint is open, including listener save-all | Refused with vanilla’s save-failed message; gate counts an open savepoint as a lifted pass; no tentative state persisted. |
| Panicking listener in each mob, dispenser and fake-player route | Action aborted and lift balanced afterwards; next valid action succeeds. |
| Listener changes target/stack and accepts | Stale action refused, listener child rolled back, no replacement placement, no published effects. |
| Independent tick after accepted placement | No additional member in frozen Multi; later update retains its normal effects. |
| No subscriber | Same states, queue order, RNG and inventory as ordinary vanilla-equivalent path; zero capture allocations. |
| Native panic/guest trap, invalid ID/count/NaN coordinate | Complete action rollback; next valid built-in action still succeeds. |

## Proposed implementation stages and open questions

File no issues or PRs from this note. Proposed PR 1: obtain public Javadoc and vanilla oracle fixtures, finish exact caller trace and freeze hook semantics/docs. With the public contract in hand, decide whether neighbour shape changes such as a fence joining belong in Multi; the fence fixture above is provisional until that decision. PR 2: start as an early draft and require design review before code of entity retirement/journal emission, block-entity attachment identity and save refusal above (review items 1–3). Add transaction-owned Sim savepoints, queue/RNG/block-entity undo and buffered publication with adversarial nested rollback tests, including a real attachment and real join number across entity removal/rollback and tentative spawn discard. PR 3: replace fixed preview with captured Single/Multi across every route, update generated API/native/wasm consumers and remove old helpers. Test each mob, dispenser and fake-player route with a panicking listener and verify a balanced lift. PR 4: run the acceptance matrix and allocation/parity measurements before declaring arismoko/horizon#855 complete.

Genuinely open: does the published counterpart require a narrower write set or a different moment? Recommendation: verify its public contract before code; if it conflicts, bring the explicit difference to review rather than relying on contributor internals. What finite journal/nesting budget is appropriate? Recommendation: select from a measured worst-case valid placement, counting spawned entities and block entities as well as block writes, with deterministic refusal at the boundary. Exact tripwire touched-position ordering remains an oracle task; recommendation: pin the five-position fixture above before implementing payload assertions. No other cancellation domain is left optional by this proposal.
