# Design: pending children

Provisional answer to [arismoko/horizon#825](https://github.com/arismoko/horizon/issues/825): the engine owns an action-local ordered collection of fully constructed, unspawned children. Listeners borrow editors; the engine validates and consumes the final collection exactly once at the original insertion boundary. This is a proposal, not an implemented API or an issue decision. Ari's side posts the decision on the issue. No implementation stages have been filed.

## Evidence and limits

Vanilla 26.3 was queried from `/workspace/horizon` using `python3 tools/rea_vanilla.py method net.minecraft.world.entity.animal.Animal spawnChildFromBreeding`. The returned method calls getBreedOffspring, returns for null, sets the baby flag and position, calls finalizeSpawnChildFromBreeding, then addFreshEntityWithPassengers. Thus the proposed callback observes a constructed child before finalization and insertion. Queries for Slime and AbstractCubeMob in the monster package failed class resolution; exact vanilla split method contents and species overrides remain unverified. Horizon's inspected owners below identify AbstractCubeMob.remove as their reference; those comments are not independent vanilla verification.

The issue names NeoForge 26.3 MobSplitEvent.getChildren and BabyEntitySpawnEvent.setChild. Requests to the public [MobSplitEvent Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/event/entity/living/MobSplitEvent.html) and [BabyEntitySpawnEvent Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/event/entity/living/BabyEntitySpawnEvent.html) failed in the web reader. Exact mutability, null-child, cancellation, parent side effects and supported breeding routes are **unverified**. The issue establishes the desired capability, not proof of NeoForge behavior. No NeoForge source, patches or other project's source was read. Public-contract and remaining vanilla verification gate implementation.

## Ownership and callback contract

Construct the complete vanilla list first, drawing inheritance, child yaw and other initialization randomness once in vanilla order. Do not finalizeSpawn or insert these children. Each entry owns its complete Mob, including opaque goal state; cloning a Mob is not an ownership transfer. Construction uses the existing unspawned-mob machinery shared with arismoko/horizon#840/#838, not a second mob store.

Provide ServerHost and a scoped collection receiver with construct, edit, replace, append and remove operations. Construct returns an unattached pending child owned by the action arena. Append/replace consumes its ownership token; it cannot be attached twice. Removal destroys the removed pending object and invalidates its editor. Unattached objects are destroyed when the callback action closes. Replacement preserves the list position, not the removed object's identity. Never accept a resident mob as a pending child. BabyEntitySpawn has a zero-or-one child slot; it supports replacement/removal, while multiple children belong to MobSplit's collection. A second baby is an error, not silently dropped.

Native and wasm use the same action identity, world epoch, slot generation and ownership checks. A callback-scoped UnspawnedMobRef is a borrow, not permission to retain or insert it later. End every borrow before nested dispatch; nested actions get separate arenas and cannot consume outer children. Reacquire a fresh editor after reentrancy. Reject expired, foreign-action, removed, already-consumed and cross-world tokens before mutation; slot reuse must never retarget a leaked handle. Earlier listeners' accepted edits are visible to later listeners in dependency order. Cancellation is monotonic.

Freeze after dispatch, validate the entire list and parent/read-set versions, then apply once. Refuse stale preconditions without recomputing breeding or replaying RNG. Ordered insertion uses the existing residency boundary and join notifications. Never retry an entered insertion consumer: retain explicit completion progress so recovery cannot duplicate earlier children. A failure before commit inserts zero; a failure after an irreversible consumer entered is a recovery error, not authorization to replay it.

## Cancellation, transactions and residency

Cancellation means zero children. Preserve the vanilla construction RNG draws already performed, and preserve the parent's death/removal for splitting. Breeding cancellation provisionally keeps age 6000 and love 0 on both parents, clears breeding credit, and emits zero child-related XP, statistics, advancements and hearts. This matches the current hook's stated policy, but its exact NeoForge correspondence remains unverified. Removing the last child uses the same outcome as cancellation. Never reconstruct children from edited counts or rewind construction draws merely because a listener vetoed.

Distinguish that veto from an enclosing transaction abort: following [multi-block placement](multi-block-placement.md), an outer abort restores its action journal, including RNG and parent mutations. Listener Host writes and accepted apply belong to one action savepoint under that coordinator. A veto discards listener writes and pending attachments while preserving the specified pre-callback vanilla construction/parent effects. A stale refusal or recoverable callback failure uses the same child suppression policy. This requires explicit savepoint boundaries; blanket rewind to before breeding is incorrect for a veto.

Follow [arismoko/horizon#1606](https://github.com/arismoko/horizon/issues/1606): pending children have no resident row, tracker entry, join number, save visibility or published attachment holder. Pending attachment data lives with its owned object using existing attachment machinery. Install it at successful residency commit; discard it on removal/veto without emitting resident Remove. On outer rollback, retire tentative residency and restore prior holders according to the placement journal. Do not publish joins or sends before root commit. Ordinary join vetoes must have an explicitly tested per-child outcome; they cannot trigger list replay.

## Owning routes and hook migration

Paths and line numbers are inspected Horizon owners at this note's base. Vanilla method names in this table are reference targets, not claims that all were independently queried.

| Route | Owner and treatment |
| --- | --- |
| Slime, magma cube, sulfur cube death | `crates/horizon-client/src/game/mobs/creatures/mod.rs:1745` split_dead_cube explicitly covers all three; replace count loop at 1754 with constructed collection. Reference AbstractCubeMob.remove. Size 1 and primed sulfur producing zero children do not dispatch. |
| Split child initialization | `crates/horizon-client/src/mobs/cube.rs:90` specializes sulfur size/baby state; yaw draws follow at 96. Preserve these draws and list order even on veto. |
| Live animal breeding | `crates/horizon-client/src/game/mobs/creatures/mod.rs:3948` breed is the common owner, reached at 3614; inheritance construction precedes finalization at 4226 and insertion at 4231/4237. Move the existing early callback after actual child construction. Include horse/donkey mule, equine inheritance, fox trust, color and variant paths in this common owner. Reference Animal.spawnChildFromBreeding and species getBreedOffspring overrides. |
| Turtle/frog eggs | Same file:4006–4015 excludes live children and preserves finalization/XP. References TurtleBreedGoal.breed and frog breeding behavior; exact vanilla frog method unverified. No baby callback for egg laying. |
| Sniffer egg | Same file:4004–4008 finalizes then lays an egg. Reference Sniffer.spawnChildFromBreeding; no live-child callback. |
| Turtle hatch | Same file:3789 hatch_turtles, reference TurtleEggBlock.randomTick. Hatching is independent spawning, outside breeding callback. |
| Spawn egg offspring | Same file:7696 spawn_egg_offspring, reference SpawnEggItem spawning behavior. Excluded from this breeding moment. |
| Villager/armadillo | Existing baby_entity_spawn declaration explicitly excludes their own paths. Exact owners and vanilla methods remain unverified; implementation must trace these before claiming full live-breeding coverage. Recommend include only if public counterpart and vanilla moment agree, otherwise document the distinct moment. |
| Native pending lend | `crates/horizon-client/src/game/modding/native_unspawned_mob.rs:107` lend_unspawned already owns scoped lending and unwind restoration. Extend its ownership discipline; do not treat the stand-in clone as a child. |

Change mob_split and baby_entity_spawn in place, once per vanilla moment. Delete count/kind-only transport, obsolete helpers and the stale “handle does not exist” prose in both declarations, hook notes, generated native/wasm bindings and samples together. Retain parent/player metadata. Do not add parallel legacy hooks or a second post-construction hook.

Coordinate with open [arismoko/horizon#2285](https://github.com/arismoko/horizon/pull/2285) (pending output FIFO, nested consumer frames and reset lifetime), [arismoko/horizon#2227](https://github.com/arismoko/horizon/pull/2227) (completed outputs across unwind), and [arismoko/horizon#2112](https://github.com/arismoko/horizon/pull/2112) (borrowed tick restoration). These are overlapping ownership/recovery foundations, not proof of atomic staged-spawn support. Their PR descriptions explicitly retain recovery limits; do not assume any supplies this collection contract.

## Trust, panic and cost

Treat all mod inputs as untrusted. Propose at most 64 split children and one breeding child; validate before allocating or consuming tokens. Reject unknown mob kinds, duplicate ownership, invalid dimensions, non-finite positions/yaw, out-of-world coordinates and invalid species sizes at the boundary. Use registered species constraints rather than a universal guessed size range. Reject an invalid operation without changing the list; a final invariant failure suppresses the action's children. Bound nesting and total arena storage; exceeding a budget refuses, never truncates.

Catch native unwinding at dispatch and handle wasm traps identically: close borrows, invalidate tokens, discard pending children and listener writes, report the listener under runtime policy. Recovery must preserve already completed unrelated outputs. Do not claim recovery from panic=abort or a second panic in restoration/destruction. No insertion runs during unwinding.

Resolve subscribers at load. With none, run today's vanilla construction/insertion directly, with zero new collection transport allocation, no registry/string lookup and no all-mod scan. Existing outer transaction requirements still apply. With listeners, work is proportional to actual children and edits. Measure both native and wasm, including allocation counts.

## Acceptance matrix

Expected implementation outcomes below are fixtures to write, not tests already passing.

| Fixture | Concrete expected value |
| --- | --- |
| Size-4 slime/magma, fixed oracle seed yielding 3 children | Three size-2 children in original order; RNG state equals vanilla after three yaw draws. Cancel: zero children, same construction RNG state, parent absent. |
| Size-4 sulfur, unprimed / primed | Two size-2 babies / zero children and zero callbacks; size-1 parent also zero callbacks. Verify against vanilla before implementing. |
| Edit/remove/append | Initial [A,B,C], edit A yaw=90, remove B, append D: commit [A,C,D] once, A yaw=90; B borrow is stale. |
| Replace | Replace C with E: [A,B,E]; C invalid, E consumed once; second append of E returns ownership error. |
| Breeding | Replace cow child with constructed pig baby: exactly one pig, age negative, no cow; both parents age=6000 and love=0. |
| Breeding veto | Zero child, zero XP/hearts/stat/advancement; both ages=6000, love=0; construction RNG unchanged from accepted construction baseline. |
| Egg exclusions | Turtle, frog, sniffer laying and turtle hatching: zero baby_entity_spawn calls. Preserve their original egg/XP results. |
| Reentrancy and targets | Nested action cannot resolve outer borrow; retained native and wasm tokens both fail after return; zero retargeted writes. |
| Invalid input | 65th split child, second baby, NaN position, unknown kind and duplicate token each fail before mutation; original valid list remains. |
| Panic/trap before commit | Zero child joins, zero published holders; pre-callback parent/RNG effects retained; unrelated completed output delivered once. |
| Outer abort | Zero children/joins/sends; original parent state, RNG and attachment values restored. |
| No listeners | Exact vanilla child fields, ordering, RNG, XP and parent effects; zero added payload allocations. |

## Proposed stages

File none. First verify the public counterpart and remaining vanilla routes and decide the questions below. Then extend the shared pending ownership boundary and native/wasm operations together. Next migrate both hooks and all authoritative routes, coordinate recovery foundations, and delete obsolete transport. Finally add oracle-backed species, reentrancy, transaction and fault fixtures plus no-listener measurements. These are proposed stages, not filed issues or authorized Rust changes.

## Open questions

- Exact NeoForge 26.3 cancellation and null-child behavior? Recommend verify public Javadoc before implementation; retain the explicit proposed parent/RNG policy until Ari decides.
- Are all live births in scope, including villager and armadillo paths? Recommend trace exact owners and counterpart call moments first; do not quietly inherit today's exclusions as proven parity.
- How does a downstream residency veto affect a multi-child list? Recommend preserve the existing per-child join refusal contract and record each completion once, while buffering externally visible effects until root commit.
- Which recovery foundation is ready? Recommend coordinate arismoko/horizon#2285/#2227/#2112 before activating pending producers; require real nested and partial-consumer tests, not an inferred blanket unwind guarantee.
- Limits and child type restrictions? Recommend 64 split children, one baby, same dimension, registered mob kinds and species-valid state; permit cross-species replacement with an explicitly baby-initialized breeding child.
