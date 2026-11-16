# Design: effect added

**Accepted with changes** for [arismoko/horizon#835](https://github.com/arismoko/horizon/issues/835), per the [maintainer decision](https://github.com/arismoko/horizon/pull/2329#issuecomment-6729847069). This remains a provisional design, not an implemented contract. The [additional requirement](https://github.com/arismoko/horizon/pull/2329#issuecomment-6729860519) withholds adoption of the pre-apply Added moment until the public NeoForge 26.3 contract is verified.

## Evidence and vanilla moment

Inspected at base `95672cb54f` using `python3 tools/rea_vanilla.py` from `/workspace/horizon`: `LivingEntity.addEffect` overload 1, `LivingEntity.onEffectAdded`, and `MobEffectInstance.update`. No NeoForge source, patches or other project's source were consulted.

Vanilla 26.3 first calls canBeAffected; refusal returns false. It looks up the active instance. An absent effect is inserted, onEffectAdded runs, changed becomes true, and the incoming instance's onEffectAdded runs. An existing instance is updated; only a true update result calls onEffectUpdated with forced=true and sets changed. Afterwards incoming.onEffectStarted runs even when update returned false. LivingEntity.onEffectAdded marks effects dirty, adds attribute modifiers and sends to passengers on the server. These are three distinct moments: first insertion, update, and accepted application attempt. Hidden-chain updates may mutate state while update returns false; false does not mean nothing changed. Stronger short effects preserve the former active instance in the hidden chain; equal-strength longer effects extend duration. Visibility/icon/ambient changes can count as updates.

The issue names [MobEffectEvent.Added](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/event/entity/living/MobEffectEvent.Added.html). The public 26.3 Javadoc request failed as inaccessible. [Public living-entity docs](https://docs.neoforged.net/docs/entities/livingentity/) were reachable but identified themselves as 26.1 and did not establish Added's 26.3 contract. Its exact timing, side, cancellation, payload and unchanged-update semantics are **unverified**. Do not infer them from its name or from vanilla onEffectAdded. Public verification is a prerequisite to implementation and may change the provisional moment below.

## Proposed ownership and contract

**Game owns the authoritative effect-add coordinator and the paused continuation.** Lower effect stores own pure applicability/update decisions and versioned apply primitives; they never depend on Game or dispatch hooks. Every authoritative producer requests this operation with a stable LivingRef, incoming full instance and optional source. The coordinator covers vanilla and registered content effects identically.

**The following moment is not adopted.** Provisionally define Added as one notification for each application admitted by canBeAffected, including replacement, extension and unchanged/hidden-chain updates. Fire after applicability, before mutating the effect store. Payload freezes incoming and previous instances, target LivingRef and optional source; provide mutable ServerHost. Added is a notice, not another applicability veto. Its return does not replace the instance. This recommendation deliberately does not equate Added with vanilla's first-insertion-only onEffectAdded. Approve only after checking the published counterpart.

Follow CLAUDE.md's decide, fire, apply rule: decide captures target identity/generation, dimension, residency and the relevant effect-store version, immutable input and exact continuation phase. Outside lifted passes, callbacks read current holders and write through Host. Inside a lifted pass, do not restore rows mid-pass: listeners read entity rows refreshed at tick exit, and listener writes queue for application after that entity's tick at its exit apply point. A heavy read of another mid-pass entity must return a narrow `Lifted` error. These read rows and the write queue are prerequisites, not existing coordinator capabilities. Apply listener writes and the stored continuation in one transaction. Revalidate after dispatch; if the target was removed, changed dimension, replaced, or its effect read set changed, refuse the stale continuation without recalculating it. Aborting that action rolls back its listener child writes. Do not overwrite the holder with a cloned pre-callback row. Unrelated actor writes survive a successful transaction.

An explicit nested add requested by a listener is a distinct operation with its own notification. Execute it in deterministic transaction order, bounded by a declared nesting budget. If it changes the outer target's read set, refuse the outer operation; never replay its notification. Nested additions merge into the outer action and roll back with it when it becomes stale. Define how coordinator refusal maps onto public operation results separately from vanilla update false, which may retain hidden-chain mutation. Count notifications at dispatch, including a subsequently refused continuation, and count committed additions separately. There is no exactly-once committed-event claim for a pre-apply notice.

Keep vanilla first-insertion behavior and effect-type onEffectAdded separate from the public Added notice because vanilla executes different work between those moments. In `game/items/potions.rs:1503`, mod_effect_added_resolved currently filters to content effects and multiplexes new-only native_on_effect_added with native_mob_effect_added. Split those responsibilities by their actual moments, remove the content filter for Added and delete duplicate caller notifications. `game/modding/native_living_effects.rs:138` must enter the coordinator instead of notifying independently. Update the catalog, generated adapters, hook notes and consumers together; retain no old numeric-ID alias.

Before implementation, verify the published counterpart's timing, logical side, mutability, cancellation, source/previous payload and unchanged/hidden-update behavior using public documentation only. Write the exact catalog signature and Host observation guarantees from that evidence. Today's catalog places Added after the first-insertion callback; moving it before mutation changes Host reads. If the verified counterpart is after mutation, preserve that public moment; internal stale tokens must not move it. Class-level coverage and vanilla algorithms do not establish this contract.

Define API-owned snapshots for incoming and previous full instances, effect keys, optional source identity and hidden chains; keep engine store types internal. Explicitly settle whether the source can be nonliving rather than narrowing it accidentally to LivingRef. Require shared-memory Wire layouts, generated coverage updates and registration/load refusal for unsupported payload capabilities; existing `Hooks::mob_effect_added` coverage does not prove redesigned payload support. Keep frozen injection-point machinery untouched.

## Route owners to migrate

Paths below are relative to `crates/horizon-client/src/`; line numbers identify inspected current owners, not promised APIs. All rows enter the same coordinator; no producer gets its own Added dispatch.

| Route | Verified owner |
| --- | --- |
| Drink | `game/items/potions.rs:987` lasting effects; instant actions at the following loop are separate from stored-effect additions. |
| Splash | `game/items/potions.rs:1054,1100` player and mob writes. |
| Lingering and cloud | `game/items/potions.rs:2060,2286,2460` player/cloud and mob application; cloud creation itself emits zero Added. |
| Beacon | `game/blocks/beacon.rs:320,322` direct primary/secondary writes. |
| Conduit | `game/blocks/block_entity_anims.rs:686` direct power write. |
| Command | `game/administration/commands/effects.rs:71,95` player and mob branches. |
| Consumption | `game/items/consume.rs:78`; mob eating and conversion behavior at `game/mobs/creatures/mod.rs:903,3827` and `game/mobs/zombie_villagers.rs:296`. |
| Mob attack and AI | `game/mobs/creatures/mod.rs:2898,3590,6439,7209,7280,7282`; wither at `game/mobs/wither.rs:227`; trading at `game/items/trading.rs:470`. |
| Enchantment | `game/items/enchant_effects.rs:545`. |
| Raids | `game/mobs/raids/mod.rs:537,731,1543`. |
| Sculk | `game/blocks/sculk.rs:1573`. |
| Sulfur | `game/blocks/potent_sulfur.rs:104,277` mob and player. |
| Other block entity | `game/blocks/block_entity_anims.rs:372` glowing. |
| Creature player effect | `game/mobs/creatures/mod.rs:4595` direct write. |
| Native handle | `game/modding/native_living_effects.rs:96,115,138`. |
| Lower mob store | `mobs.rs:4715,4731,5024` wrappers and instance update; convert gameplay callers to resumable decisions, not a callback reaching upward. |
| Player store | `game/items/potions.rs:1409,1422` applicability and update. |

Before migration, classify every producer by whether its actual call path runs inside a lifted pass. Commands, beacon, conduit, drink and splash from item use, and consumption outside passes migrate first. Mob attacks, raids, sculk and other mob AI wait for entity read rows and the write queue; consumption reached from mob AI belongs to that later group. Audit remaining routes rather than assuming their phase. Extend the census to turtle helmet, witch, illusioner, axolotl, contact effects, pufferfish and lower consumable paths.

This census identifies write owners, not a completed vanilla caller parity proof. Initialization/deserialization, network replication and test setup must be classified explicitly: restoring an already-active effect emits zero Added. Audit direct store mutations and trial-spawner initialization (`game/mobs/trial_spawner.rs:353`) before implementation; do not silently turn load/spawn restoration into application attempts.

All living holders use stable handles: local authoritative player, LAN guest, ordinary mobs, bosses and living holders currently refused by native writes (armor stands and dragon). Preserve vanilla immunity/override behavior; do not broaden acceptance merely to provide a handle. Resolve holder capabilities through the residency table lookup: dragon belongs to the design's 20 residency kinds and must not acquire a native-only exclusion. Verify armor stand applicability through the same capability model. Native API currently refuses these holders (`native_living_effects.rs:90`); holder capability parity requires explicit verification. Nonliving entities cannot be targets. Integrated single player, LAN host/guest and dedicated server use the same authoritative coordinator; remote prediction and inbound effect packets emit zero server Added. Exercise real guest routing rather than treating temporary player context as holder identity.

## Residency, safety and cost

Use the existing residency boundary from [arismoko/horizon#1606](https://github.com/arismoko/horizon/issues/1606) and the [placement design](multi-block-placement.md). Within a pass, retain lifted ownership and use the read rows and queued writes at the entity exit apply point; reacquire by generation there. Do not require restoring all lifted rows before dispatch or describe the planned write queue as already built. Balance lifted-pass leases on success, refusal and unwinding; no duplicate raw entity-list path. Preserve listener-spawned entities and attachment/join identities; defer retirement and publication to root commit. Saving while a lifted pass/action is open must refuse rather than persist partial state.

[PR arismoko/horizon#2297](https://github.com/arismoko/horizon/pull/2297) and [PR arismoko/horizon#1863](https://github.com/arismoko/horizon/pull/1863) were OPEN when inspected. The former carries the latter and modifies consume, fox and native effect integration. Neither establishes complete effect-add unification. Coordinate with their paused mob continuations and preserve their ownership restoration; recheck merged owners before implementation. Do not introduce a competing mob lift. [PR arismoko/horizon#2112](https://github.com/arismoko/horizon/pull/2112) is also OPEN; its mob-listener unwind restoration and stack are dependencies to coordinate with, not merged guarantees.

Mod input is untrusted: validate registry ownership, handle generation, dimension, duration (-1 infinite or nonnegative ticks), amplifier clamped to 0..=255 and flags at the handle boundary. Reject duration below -1 and unknown effect keys without changing unrelated no-op behavior. Validate finite numeric inputs before conversion where an adapter accepts floats; measure hidden-chain/nesting budgets without rejecting vanilla-valid instances or silently truncating valid state. Catch native unwinding and guest traps at dispatch, abort the action transaction, restore lifts and use runtime failure reporting/disable policy. Process-aborting panic cannot be recovered.

Resolve subscribers at load. No listeners means no payload allocation, cloning of hidden chains, event queue or per-tick scan; use the same vanilla update primitive directly with existing transaction guarantees. Subscribed cost is proportional to listeners and the touched effect chain. Never scan every mod or every living holder to find additions.

## Acceptance matrix

Required future tests, not validation claimed by this note. Counts follow the provisional admitted-attempt definition above.

| Fixture | Concrete expected values |
| --- | --- |
| Each route above, fresh speed 200 ticks amplifier 0 | Exactly 1 Added, target exact LivingRef, previous=None, final speed=(200,0). Use route-specific effect where fixed by vanilla: conduit_power, darkness, nausea, poison, wither or glowing. |
| Existing speed=(200,0), incoming=(400,0) | 1 Added, final=(400,0), zero first-insertion effect-type callbacks. |
| Existing speed=(200,0), incoming=(100,0) | 1 Added, final=(200,0), return false. |
| Existing speed=(200,1), incoming=(400,0) | 1 Added, active=(200,1), hidden=(400,0), return false; hidden mutation retained. |
| Stronger short speed=(40,2) over (200,0) | 1 Added, active=(40,2), previous retained hidden with duration 200. |
| Applicability refuses poison on immune target | 0 Added, no inserted poison, return false. |
| Listener removes target or changes its effect version | 1 dispatched Added, 0 committed outer additions; stale write refused, no retarget to recycled ID. |
| Listener explicitly adds another effect | 2 notifications total; both apply if independent; same-effect nesting invalidates outer continuation. |
| Listener changes another mob, moves actor and spawns a mob | Changes survive successful apply; no old row restoration erases them; attachments/join numbers preserved. |
| Panic/trap; invalid duration -2; unknown key | Panic rolls action back and balances lift; invalid inputs yield 0 Added. Next built-in application succeeds once. |
| Each producer inside a lifted pass, listener panics | Action rolls back; existing mob-listener restoration runs; lifts balance on unwind, refusal and success; next entity/application remains usable. |
| In-pass reads and writes | Read rows reflect tick-exit refresh; heavy read of another mid-pass entity returns `Lifted`; writes apply only at the owning entity exit point. |
| Listener spawn/removal and save during open action | Publication and retirement wait for root commit; rollback publishes/retires neither; save refuses even without an open lift. |
| Holder capabilities and payload transport | Dragon resolves through residency lookup; armor stand applicability is verified; native and guest snapshots preserve chains/source identity; unsupported capabilities refuse registration/load. |
| Verified public moment | Host observations and exact catalog signature match published timing, side, mutability and cancellation; pre-apply moment remains unadopted until verified. |
| Integrated, LAN guest and dedicated | Same fixture yields 1 server Added each; replication yields 0 additional Added. |
| Load/replication/instant damage with no stored add | 0 Added. |
| No subscribers | Same active/hidden state and return values as ordinary update; 0 event allocations. |

## Proposed implementation stages

File no issues from this list.

1. **Before any code, verify the public NeoForge 26.3 Added contract.** Confirm timing, cancellation, payload, side and mutability using public docs only; record the exact signature and observation guarantees. The pre-apply moment is not adopted until then. Complete the vanilla caller census, phase classification and holder capability fixtures.
2. Introduce pure lower-store decisions and the Game-owned coordinator with stale checks for producers outside lifted passes first: commands, beacon, conduit, item use and consumption outside passes. Require payload/Wire coverage and refusal behavior before exposing the API.
3. Wait for entity read rows and the write queue before migrating mob-pass producers, including attacks, raids, sculk and other mob AI. Apply queued writes at entity exit and reuse accepted mob-listener unwind restoration; never restore rows mid-pass or migrate these producers first. Remove duplicate/content-only dispatch as each route migrates and update generated API/docs atomically.
4. Run the full route/holder/server matrix, per-producer in-pass panic and balanced-lift fixtures, nested rollback/result mapping, save refusal and allocation/budget measurements before claiming arismoko/horizon#835 complete.

## Open questions

- What exact moment does published 26.3 Added cover? Recommendation: admitted application attempts, including unchanged updates, provisionally; verify Javadoc before implementation and follow Ari's decision if different.
- Can a pre-apply Added be observed for a refused continuation? Recommendation: yes, explicitly distinguish dispatched from committed counts; never retry a stale decision. If committed-only notice is required, redesign the moment before coding.
- How broad must holder capability support be? Recommendation: every vanilla living holder with its real applicability overrides; verify armor stand/dragon behavior rather than retaining blanket native exclusions by accident.
- What nesting and chain budgets apply? Recommendation: deterministic runtime-wide measured limits with transaction refusal, shared with existing Host operations; no silent truncation.
- What exact catalog payload, source identity (including nonliving sources), Wire layouts and public refusal result follow from verification? Requirement: settle these before exposing code; reject unsupported capabilities at registration/load.
- When may in-pass producers migrate? Requirement: only after read rows, write queue and shared unwind restoration exist; outside-pass producers go first.
- What may merge ahead of the carry PRs? Recommendation: this docs note only; implementation must integrate the accepted continuation ownership and world-visibility decision.
