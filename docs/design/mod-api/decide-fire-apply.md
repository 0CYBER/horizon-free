# Design: decide, fire, apply

**Status: accepted with changes** by the [maintainer decision](https://github.com/arismoko/horizon/issues/784#issuecomment-6729846827), combining both review views. The requirements below must be settled before runtime code; the note remains a provisional design, not implemented behavior.

Contract for [#784](https://github.com/arismoko/horizon/issues/784), for Ari's review and decision on the issue. This is a documentation proposal, not implemented behavior. The named question is the decision token and stale-apply semantics across client-lib and menus.

## Evidence and limits

Vanilla 26.3 was inspected from `/workspace/horizon` with `python3 tools/rea_vanilla.py method net.minecraft.world.inventory.AbstractContainerMenu clicked`. Its entry calls `doClick(slotIndex, buttonNum, containerInput, player)` synchronously. Exceptions become a reported crash with menu type, class, slot count, slot, button and input type. This verifies the entry and exception boundary only; it does not establish a reusable decision token, atomic rollback, or all downstream click semantics. Downstream vanilla callers and individual station algorithms remain to be pinned before runtime migration.

The issue explicitly names **no NeoForge 26.3 counterpart (Horizon design)**. The permitted [26.3 AbstractContainerMenu Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/minecraft/world/inventory/AbstractContainerMenu.html) could not be accessed by the web reader. NeoForge's downstream click, cancellation and transaction behavior is consequently **unverified**; no equivalence is claimed. No NeoForge source, patches or other project's source were consulted. Public docs must supply any future counterpart claims.

## Proposed contract

Client-lib decides using an immutable view; game dispatches the existing named hook with the catalog-selected Host; client-lib applies the frozen plan after validation. Lower crates never construct a Host or depend on Game. This is a control-flow boundary, not three new public hooks. Retain one hook per vanilla moment, and remove replaced transports and all callers together without compatibility aliases.

A decision is an internal, opaque, non-cloneable, single-use value. It contains action kind, side, actor lifecycle holder, world/session identity, target lifecycle holder, transaction/savepoint identity, proposed ordered mutations/effects, and a typed read set. Menu decisions additionally bind the menu instance generation, container identity, selected recipe and registry generation, slot layout/rules generation, cursor, input slots, output slots, capacities, permissions, costs, drag state and backing-storage versions actually consulted. Full stack components participate, not just item IDs/counts. Capture RNG state/draw position where relevant; decide must not consume shared RNG or publish effects. Store planned draws or reserve them transactionally. Never expose the token as mod-controlled bytes or retain borrowed menu pointers across dispatch.

Versions advance on every relevant mutation, including changes restored to equal values (ABA). The read set is operation-specific: an unrelated slot outside it need not invalidate a decision. Global identity checks always apply. Complete mutation epochs are required even in the interim: exact snapshots cannot detect ABA. Where per-slot mutation coverage is incomplete, conservatively invalidate the whole menu with a complete menu mutation epoch; snapshots may supplement, never replace, epochs. Do not silently accept an incomplete read set for performance.

Apply returns a proposed typed outcome: `Applied`, `Canceled`, `Stale(reason)`, `InvalidInput(reason)` or `ListenerFailed`. `Busy` is an admission error before token creation, dispatch or mutation, not an apply outcome. Existing public operation results remain the only public result API: `Applied` maps to the existing success result; `Canceled` to that hook’s existing cancellation result; `Stale` and admission `Busy` to the operation’s existing refused/no-change result; `InvalidInput` and `ListenerFailed` to its existing failure/refusal result, with runtime diagnostics. Preserve each operation’s existing return shape and cancellation semantics; export neither tokens nor a second transaction API. A no-action decision dispatches nothing. Validate once before firing and again immediately before mutation against the transaction's live overlay, including listener writes. Deferred actions validate again at their ordered execution position. Invalid identity, changed cursor/slot/rule/recipe/permission or invalidated world refuses the original operation. Never rerun decide, retarget, reselect a recipe, repeat dispatch or retry automatically. A later player input can request a new operation. Consume the token on every terminal outcome; a second apply is invalid input.

Entity identity uses the existing lifecycle holder (server-wide epoch plus id), resolved through the entity table lookup as `Found`, `Taken` or `Missing` (#1606). No second entity generation is introduced. `Missing` is stale; `Taken` at admission is `Busy`, or during apply is `Stale`, never a retry.

Hook-specific mutable output is distinct from token preconditions. Validate that output against the unchanged decision inputs, then apply it once. For example, an anvil output override is allowed by its hook; replacing the input stack via Host invalidates the decision. Preserve each existing hook’s name, side and cancellation meaning rather than assigning a universal item-loss policy.

## Transactions and reentrancy

Open an action child savepoint under the existing Host transaction before decide. Listener writes and apply share that action boundary. Nested listener commits merge into the action, never publish independently. Cancellation, stale refusal, invalid output or recoverable failure aborts the action, including listener writes, inventories, cursor, costs, RNG, deferred drops, notifications and packets. Outer abort also undoes an accepted action. Root commit alone publishes effects. Inside a lifted pass, defer drops, entity spawns, attachment-holder retirement and journal emission to root commit, following the placement note (#2319); rollback cannot un-retire a holder. Preserve its rollback ordering: discard listener pending writes/effects, undo listener Sim writes, reverse action mutations, restore actor/inventory/RNG, then close aborted. Listener reads of other entities use entity rows refreshed at tick exit; tentative entity changes remain invisible until commit.

An open action savepoint counts as a lifted pass for the save gate (#2211 and follow-up). Refuse every save during it, including listener `save-all`, with the existing save-failed message; the lease foundation alone is not the save gate. Use `game/runtime/transactions.rs:174,179,303,399,462,513` for the existing open/close, menu journal, deferred effects, commit and rollback owners; these facilities are foundations, not proof of a complete menu/Sim journal.

Reentrant close/replace or same-menu click invalidates the outer decision. Refuse recursive execution on the active menu with `Busy`, before dispatch or mutation; allow other-object work only under nested savepoints. A requested close may be staged, but if it invalidates the outer action the whole action aborts, including that close. Do not retain a mutable borrow while firing. Restore lifted storage and release guards on all unwind paths. Multi-step quick move and recipe placement are one action with an ordered plan; no partial successful prefix on refusal. Post-commit notices remain separate moments and cannot retroactively veto committed writes.

## Scope and inspected owners

Paths below are relative to this worktree's base. These are verified existing entry owners, not claims that their complete transitive caller graph is already migrated. The implementation must finish the vanilla caller oracle and per-route trace before claiming #784 complete.

The current per-route evidence is in [route inventory](decide-fire-apply-routes.md): every generated client-lib target, each menu route's current owner with worktree line numbers, its read set, the existing public result each outcome maps to, the vanilla methods checked, and the open items for stage 2.

| Hooks or route | Current owner and proposed treatment |
| --- | --- |
| `item_stacked_on_other` | `crates/horizon-client/src/menu.rs:384,3422`; authoritative hub path `crates/horizon-client/src/lan/menu.rs:926`. Freeze cursor and both stacks before dispatch; apply one validated answer. |
| `menu_slot_rules` | `crates/horizon-mod-vocab/src/hooks/base.rs:2123`; `crates/horizon-client/src/menu.rs:610,622,1894,2596`; LAN `crates/horizon-client/src/lan/menu.rs:469,658`. Rule answers become plan inputs; no recursive hook invocation during validation. |
| Local menu click, cursor, quick move, drag | `crates/horizon-client/src/menu.rs:3208,3589,3600,3626,3642`; share one coordinator, including throw/swap and output-taking branches of click. Preview remains read-only and cannot commit or dispatch authoritative server hooks. |
| LAN authoritative clicks/backing storage | `crates/horizon-client/src/lan/menu.rs:784,227,283`; bind authoritative guest/menu generations and validate backing storage before write-back. Dedicated and single-player server routes use this same contract. |
| Recipe-book and mod crafting placement | `crates/horizon-client/src/game/network/lan_crafting.rs:789,974`; bind recipe, all selected source slots and destinations; refuse changed inputs without choosing replacements. |
| Anvil and grindstone prepare/take | `crates/horizon-client/src/menu.rs:1389,1401,1412,1425,1440,2059,2073,2084`; migrate paired query/write-back helpers to tokens without merging distinct prepare and take moments. |
| Menu close and merchant state | `crates/horizon-client/src/menu.rs:3794,3865`; invalidate generations and journal cursor/inventory settlement. |
| Motion hooks: living_jump, living_jump_power, living_speed, living_flying_speed, block_jump_factor, block_speed_factor, player_stays_on_ground_surface, fluid_flow | `crates/horizon-client/src/motion.rs:52,59,67,75,83,95` and `motion/surface.rs:48,56`. Out of scope: retain value-in, value-out fires with Host. Tokens/read sets add per-entity per-tick cost and provide no stale-apply protection for pure value transforms. |
| Other catalog client-lib targets | `crates/horizon-mod-vocab/src/hooks/base.rs:2520,3424,4781,4793,4805,4815` owns living_breathe, ingredient, living_drown, living_fall, living_heal targets (living_jump is excluded above). `hooks/ingredient_test.rs:15`, `recipe_property_sets.rs:20`, `ender_entity_teleport.rs:14`, `farmland_trample.rs:13`, `player_name_format.rs:14`, `player_tab_list_name_format.rs:15`, `hooks/entity_move.rs` own remaining explicit client-lib targets. Each needs a typed decision or an explicitly read-only fire boundary, not a Host-less escape hatch. The read-only exception is only a dispatch shape; every hook still receives Host — except where a maintainer ruling says otherwise: `entity_move` (#2414) is a loop-context hook without a host whose listener only decides the delta; its result validation and panic poison replace the stale-decision recheck. |
| Generator/game dispatch | `crates/horizon-mod-api-gen/src/lib.rs:547` owns trait generation; `crates/horizon-client/src/game/native_hooks/fire.rs:5` includes generated dispatch. Generate typed side/Host signatures and native/wasm adapters from one catalog. |

Remote prediction never gains server authority. Client-side hooks receive ClientHost; server menu mutations receive ServerHost; both-side moments use typed HostSide. Network corrections may be sent after refusal to reconcile prediction, containing authoritative pre-action state; no success effects escape. Non-menu targets above are migration scope, but their precise read sets require individual oracle fixtures; this note does not invent their vanilla algorithms.

## Trust, panic, target parity and cost

Validate untrusted mutable hook outputs and Host requests at the callback boundary. Internal typed values guaranteed by construction need no repeated structural validation in apply; apply rechecks live preconditions only. Validate owned IDs, generations, enum discriminants, slot indices, stack components/counts and capacities; reject non-finite numeric outputs and checked-arithmetic overflow. Preserve existing documented clamps where the hook owns them; do not turn unrelated unknown IDs from no-ops into errors. No arbitrary token deserialization, pointers or caller-supplied version numbers. Set finite read-set, plan-size and recursion budgets before runtime rollout; overflow refuses atomically, never truncates a plan.

Menu routes require #2232 and tracking issues #2244 and #2240 before migration: clicks run under `as_guest`, and a panicking listener must leave the guest swap restored.

Catch native unwinding at game dispatch; guest traps use the same ListenerFailed outcome and rollback. Log/report the failing listener under runtime policy. Process-aborting panics and rollback failure cannot be promised recoverable. A subsequent built-in action must work after a caught failure. Native and wasm expose the same hook/Host contract and ordered dependency dispatch; wasm handles are scoped, checked resources, not transferable decisions. Any changed public payload requires a shared-memory `Wire` representation and generated coverage row. Unsupported capabilities must fail registration/load with a named `UnsupportedHookCapability` refusal identifying the hook/capability, for both targets. Internal tokens need no Wire form. Leave frozen injection-point machinery untouched. Update generated bindings, docs and samples together.

Resolve subscribers at load. With no listeners, take the ordinary operation directly with zero token/read-set/payload allocations and zero all-mod scans; preserve enclosing transaction guarantees. Subscribed cost is proportional to consulted state and planned writes. Validation reads stored rule results and versions; it must not re-fire callbacks or recalculate an alternative action.

## Acceptance matrix for implementation

These values are proposed requirements, not passing runtime tests. All refused cases publish zero success packets, drops and costs.

| Fixture | Expected value |
| --- | --- |
| Cursor stone 3, empty slot, accept ordinary placement | Cursor 0, slot stone 3, one apply and one dispatch. |
| Same fixture, listener cancels after writing unrelated slot dirt 2 | Cursor stone 3, destination empty, unrelated slot restored, zero apply. |
| Listener changes cursor to stone 4 and allows | Stale; cursor restored to stone 3, destination empty, one dispatch, zero recomputations. |
| Input stone 3 changes to dirt 3 then back within listener | Stale by version; original state restored; zero apply. |
| Menu generation 7 replaced with identical generation 8 layout | Stale; no write to replacement; replacement rolled back with action. |
| Quick move stone 10 into capacities 4 and 6 | Accept gives 4 and 6, source 0; listener changes either capacity: entire move refused, source 10. |
| Recipe needs two stone; source stone 2, output empty | Accept consumes 2 once; listener removes one input: stale, source restored to 2, output empty. |
| Nested transaction commits dirt 2, outer listener traps | Dirt write rolled back; next ordinary stone placement succeeds. |
| Reentrant same-menu click | Inner admission Busy with zero token creation or dispatch; outer proceeds only if unchanged. |
| Entity table returns Taken | Admission Busy or apply Stale; zero retry or retarget. |
| Spawn/drop then cancel in lifted pass | No spawn, retirement or journal publication; existing holders survive. |
| Save during action savepoint | Refused; no tentative state persisted. |
| Guest listener panics | Guest swap restored, action aborted; next built-in action succeeds. |
| Listener reads another tentatively changed entity | Reads tick-exit row until commit. |
| Changed public payload / unsupported capability | Wire and generated coverage on both targets / named load refusal. |
| Motion value transform | Existing Host-bearing value fire; zero token/read-set allocations. |
| Duplicate apply/token from another session | InvalidInput, zero mutations. |
| NaN, infinity, invalid slot, overflowing count from listener | InvalidInput and complete action rollback; next built-in step succeeds. |
| Outer transaction aborts after Applied | Original cursor, slots, costs and RNG restored; zero published effects. |
| Native versus wasm, identical listener sequence | Same outcome, stacks, effects and dispatch order. |
| No subscriber | Same final state/RNG/effect order as baseline; zero decision allocations. |

## Proposed implementation stages

File no issues or PRs from this note.

1. Before code (PR 1), confirm downstream NeoForge menu click, cancellation and transaction contracts from public method/event documentation before assigning counterpart contracts to individual hooks; inventories and class-level coverage do not establish semantics. Finish downstream vanilla oracle/caller fixtures and the accepted token, refusal and rollback requirements on #784. Inventory every generated client-lib target, including targets without explicit annotations.
2. Add internal typed tokens, mutation generations and action savepoints to the existing coordinator; prove rollback and overlay validation with adversarial fixtures before exposing callbacks.
3. After #2232, #2244 and #2240 guest restoration prerequisites, migrate menu routes together, coordinating #2318 capacity ownership, #2259 mutable argument write-back and #2249 transaction fixtures. Remove old transports and update generated native/wasm bindings and samples.
4. Excluding the eight motion value transforms, migrate remaining client-lib targets in bounded slices with their individual read sets and side contracts. Run parity, failure recovery and allocation measurements before closure.

This unblocks the Host-bearing client-lib/menu half of the clean break, and supplies the stale-action foundation for #855 multi-block placement and #731 screen initialization where applicable. It does not settle #781 attachment Data storage, #792 entity storage or #796 identity by itself; those contracts supply identities this mechanism must validate.

## Open questions

- **Version granularity:** whole menu or consulted slots? Per-slot/rule epochs plus mandatory menu generation are preferred; complete whole-menu mutation epochs are required until per-slot coverage is complete. Only the eventual granularity remains open; snapshots alone are never sufficient.
- **Reentrant close semantics:** should a listener close survive refusal? Atomic rollback with the action and admission Busy for recursive same-menu execution are required here; confirm operation-specific public refusal mappings in each route inventory before code.
- **Budgets:** which numeric limits? Recommend measuring maximum valid recipe/menu plans first, then freezing explicit limits and boundary tests; no arbitrary guessed limit in this proposal.
- **Legacy clamp versus rejection:** recommend retaining each hook's documented range/clamp and rejecting malformed structural output. Review each migrated declaration rather than imposing a new universal numeric policy.
- **Complete route evidence:** downstream vanilla and implicit generated targets remain unverified. Recommend making the route inventory and individual oracle fixtures a merge prerequisite for runtime implementation; this accepted provisional note is not evidence of completion. NeoForge downstream menu contracts remain unverified and public documentation confirmation is a PR 1 prerequisite.
