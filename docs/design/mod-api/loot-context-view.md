# Design: Read-only typed loot context

Provisional proposal for [arismoko/horizon#2647](https://github.com/arismoko/horizon/issues/2647). Docs only; approval precedes code. Broaden the existing `LootContext` snapshot at the condition/function evaluation boundary. Do not provide a mutable Host, live world access, or another loot hook. [arismoko/horizon#2648](https://github.com/arismoko/horizon/issues/2648) owns the narrow tags view. [PR arismoko/horizon#2595](https://github.com/arismoko/horizon/pull/2595) owns `LevelRef::roll_loot` and its FISHING-shaped input; this proposal neither expands that input nor changes random ownership.

## Evidence and current routes

The [public NeoForge loot-table documentation](https://docs.neoforged.net/docs/resources/server/loottables/#loot-parameters) describes typed `ContextKey<T>`, presence tests, required/optional parameter sets, and unregistered custom keys. Its rolling context also contains level, random, luck and dynamic drops. These are distinct from parameters. The page is not pinned to NeoForge 26.3: NeoForge-specific 26.3 behavior is **unverified**, including custom-key registration and exact set membership. The owner-supplied read list is a later review gate for the subset below.

Vanilla 26.3 bytecode can be inspected with `python3 tools/rea_vanilla.py class net.minecraft.world.level.storage.loot.parameters.LootContextParams` and the corresponding `LootContextParamSets` class. All fifteen requested constant names exist. Important type details: TOOL is `ItemInstance`, ADDITIONAL_COST_COMPONENT_ALLOWED is `Unit`, not boolean, and vanilla additionally exposes CONTAINER (`SlotProvider`), absent from the public page's list. Field inspection confirms names and types, not required/optional membership. The set memberships below follow vanilla's parameter set declarations for the sets named; membership of the remaining sets is verified in stage one.

Verified repository-relative owners:

| Route | Owner |
| --- | --- |
| Existing API snapshot and nested Wire values | `crates/horizon-mod-vocab/src/world/snapshots.rs:104`, `:129`, `:140`, `:174`, `:187`, `:199` |
| Condition/function catalog, authoritative server | `crates/horizon-mod-vocab/src/hooks/loot_condition.rs:9`, `loot_function.rs:10` |
| Guarded conversion and dispatch, immutable context | `crates/horizon-client/src/game/modding/api_values/loot_values.rs:7`, `:24`, `:174` |
| Engine callbacks and borrowed context | `crates/horizon-world/src/loot.rs:89`, `:103`, `:120`; forwarding `crates/horizon-world/src/native_load_hooks.rs:40`, `:51` |
| World evaluator: custom condition, context capture, custom function | `crates/horizon-world/src/loot.rs:1583`, `:1613`, `:2107` |
| Items evaluator: context capture, custom condition/function | `crates/horizon-items/src/items.rs:2049`, `:2234`, `:2627` |
| Place scope shared by both evaluators | `crates/horizon-world/src/loot.rs:187`, `:200` |
| Existing typed block value | `crates/horizon-mod-vocab/src/block/block_state.rs:9` |

Both evaluator routes must carry the same typed slots through the common boundary. Blocks, containers, fishing and command-driven item rolls reach the items evaluator; entity drops and world loot reach the world evaluator. Caller-specific context must be carried explicitly, never reconstructed from whichever Game fields happen to be selected. Before implementation, enumerate gameplay producers and prove their parameter-set selection with fixtures; these evaluation owners do not establish complete producer coverage for interaction/enchantment contexts currently missing from the snapshot.

## Parameter audit and proposed subset

R means required in the named set; O means allowed but optional. A parameter is forbidden in sets that do not allow it; generic validation is not permission to invent values. The examples below describe the proposed initial supported sets, BLOCK, ENTITY and FISHING, plus the public-doc families relevant to deferred keys. Exact remaining set membership is a stage-one verification prerequisite, not inferred from a parameter's name.

| Parameter | Snapshot today | Proposed API-owned typed form | Set semantics | wasm |
| --- | --- | --- | --- | --- |
| this_entity | `entity: Option<LootEntity>` predicate facts | `LootEntity` | R ENTITY; O BLOCK/FISHING | Existing facts cross; no live entity |
| interacting_entity | Missing | `LootEntity` | O ENTITY_INTERACT/BLOCK_INTERACT; absent initial sets | Deferred capture, then Wire |
| target_entity | Missing | `LootEntity` | R ENTITY_INTERACT (which also requires tool); absent initial sets | Deferred capture, then Wire |
| last_damage_player | `player` predicate facts | `LootEntity` constrained to player | O ENTITY | Existing facts cross |
| damage_source | `damage: Option<LootDamage>` | `LootDamage` | R ENTITY | Existing facts cross; no live DamageSource |
| attacking_entity | Nested `damage.source`, not independent presence | `LootEntity` independent slot | O ENTITY; not allowed in FISHING | Facts cross; independent capture needed |
| direct_attacking_entity | Nested `damage.direct`, not independent presence | `LootEntity` independent slot | O ENTITY | Facts cross; independent capture needed |
| origin | `place.origin` | `[f64; 3]` | R BLOCK/ENTITY/FISHING | Already crosses |
| block_state | `block` identifier only; loses properties | Existing `BlockState` | R BLOCK | Wire exists; producer conversion missing |
| block_entity | `place.block_entity: Option<Tag>` saved metadata | Existing API `Tag` snapshot | O BLOCK | Already crosses as NBT; live object does not |
| tool | `LootTool`: item and enchantments only | Existing `LootStack` snapshot of instance | R BLOCK/FISHING, including empty instance | Wire exists; full instance capture missing |
| explosion_radius | Missing even though items evaluator has it | `f32` | O BLOCK | Deferred today; proposed initial subset |
| enchantment_level | Missing | `i32` | R enchantment sets and HIT_BLOCK; absent initial sets | Deferred producer, scalar is wireable |
| enchantment_active | Missing | `bool` | R ENCHANTED_LOCATION; absent initial sets | Deferred producer, scalar is wireable |
| additional_cost_component_allowed | Missing | `()` presence token | R VILLAGER_TRADE; absent initial sets | Deferred producer; never encode as false |
| container (additional vanilla key) | Missing | No suitable snapshot chosen | R CONTAINER_PROCESS (with block_entity, block_state and origin) | Unsupported: live SlotProvider has no read-only API projection |

These are deliberately snapshots, not claims of full Java object equivalence. Entity properties unavailable in `LootEntity`, arbitrary entity methods, block-entity methods, live inventory access and damage-source methods cannot cross today: the bridge has only the listed captured Wire facts and no safe read backend during split level work. Native receives exactly the same subset. Do not expose an extra native pointer. Existing entity handles, if added later, must use the existing lifecycle holder identity with no second generation; identity alone grants no reads.

Build first: independent this/attacker/direct/player facts, damage facts, origin, typed block state, saved block-entity Tag, full tool snapshot and explosion radius for BLOCK/ENTITY/FISHING. Retain luck, biome, seed and captured level facts as context metadata, not fabricated ContextKeys. Keep the engine's captured seed convention; do not expose its mutable random or consume extra draws for new slots. Interaction, enchantment, villager-trade and container producers are later work, pending verification and real-read demand.

## Typed view and registration

Extend the existing API context in one clean break; remove renamed/redundant fields and update every caller, generated boundary, sample and catalog together. No parallel legacy context. Proposed `LootContext::get(&LootKey<T>) -> Result<Option<&T>, LootContextReadError>` and `require` distinguish absent optional (`None`), disallowed (`ParameterNotAllowed`), undeclared (`NotDeclared`) and unsupported (`UnsupportedLootParameter`). A read requires declaration, as vanilla validates the parameters a function references against the table's parameter set: an undeclared read answers the typed `NotDeclared` error, never the value and never a `None` that looks like an absent optional. Missing required values refuse evaluation as `MissingRequiredLootParameter` before invoking a mod. A typed key cannot be constructed with an arbitrary conflicting T. Presence is explicit: an empty required tool is present, zero radius and false enchantment activity are present, and Unit presence differs from absence.

`LootContext`, proposed key/set descriptors, error enums and every new crossing snapshot explicitly use `#[derive(Wire)]` (the repository macro is `horizon_mod_api_macros::Wire`). Existing `LootEntity`, `LootDamage`, `LootStack`, `LootPlace`, `LootLevel` already derive it; `BlockState` also derives it. Use the existing API Tag representation and shared NBT codec. No engine types, Java objects, serde values as typed parameter payloads, arbitrary `Any`, or manual per-target encoding. Generate and validate native/wasm dispatch together.

Mods declare every parameter they read, required or optional, when registering a loot kind; a read outside the declaration is `NotDeclared`. Unknown built-in keys or deferred keys refuse registration with proposed `UnsupportedLootParameter { key }`; unsupported set families refuse as `UnsupportedLootParameterSet { set }`. A kind's required reads must be allowed by its declared sets, otherwise `LootParameterSetMismatch`. Optional reads may be absent, but unsupported capability must never masquerade as absence.

Defer custom keys and custom sets in this first build. Proposed `UnsupportedCustomLootKey` and `UnsupportedCustomLootParameterSet` are load-time refusals on both targets. Later, permit namespaced load-time declarations with a fixed API-owned Wire schema, unique owner, duplicate/type-conflict refusals, and explicit producer binding plus required/allowed set descriptors. Freeze resolved numeric slots before world use; no runtime string-map registration. The public docs' unregistered Java keys do not establish a safe cross-process type contract for Horizon.

## Safety, cost and action boundaries

This view takes no mutable Host and cannot mutate world, save, run dynamic drops or recursively roll loot. Existing hooks remain their current moments; this is a value contract extension. Capture on the authoritative server before entering mod code and release engine borrows. Owned snapshots remain stable across nested evaluation. Scope restoration must hold on unwind.

Treat declarations, JSON and function output as untrusted. Resolve key/set identifiers at registration; reject type conflicts and invalid registry keys. Reuse shared Wire frame, allocation and NBT budgets cumulatively across the entire context, including equipment and vehicle chains; NBT container depth may reach vanilla's 512 limit, not a new smaller arbitrary cap. Before code, verify shared byte limits against largest legal vanilla instances and raise transport capacity where needed. Reject oversized frames explicitly as proposed `LootContextBudgetExceeded`, never truncate facts or report missing optional data. Origin/radius must be finite; radius nonnegative. Preserve signed i32 enchantment levels and candidate counts rather than imposing inventory materialization rules here. Function output remains staged and fully validated before one writeback; refused conversion leaves the candidate unchanged.

Proposed failure policy: a caught callback panic or wasm trap discards that callback's staged output, attributes the fault to the mod, and aborts this evaluation with a diagnostic; never partially apply it. Do not claim current runtime containment merely because dispatch tracks attribution. Condition defaults and function output must not turn a malformed required context into a successful roll. No new writes, retirement or journals originate in the view. Existing enclosing actions retain root-commit journal/retirement publication inside lifted passes and must refuse saves while an action savepoint is open.

No listener: retain the existing constant-time subscription guard before conversion; no parameter map allocation, serialization, NBT copy or additional RNG draw. With listeners, capture cost is proportional to declared captured values and encoded bytes; fixed resolved slots make reads O(1). Both evaluator guards and their seed-producing callees must be measured, not just the game adapter.

## Acceptance matrix

Run every fixture through an actual custom condition and function on native and wasm and compare typed observations, not just codec round trips. Expected values below are proposed producer contracts.

| Fixture | Concrete expected observations |
| --- | --- |
| Adult zombie at (10,64,20), directly killed by player P using iron sword | ENTITY: this.kind=`minecraft:zombie`, baby=false; origin=[10,64,20]; player.kind=`minecraft:player`; attacker and direct attacker both P's captured facts; damage.kind=`minecraft:player_attack`. TOOL disallowed in ENTITY: do not infer sword from attacker equipment. Block/radius absent/disallowed. |
| Same zombie killed by P's arrow | Attacker=P; direct.kind=`minecraft:arrow`; damage.kind=`minecraft:arrow`; last_damage_player=P. |
| Zombie falls without player attribution | Required this, origin and damage (`minecraft:fall`) present; player/attacker/direct absent. |
| Stone at (2,70,4), bare-hand block roll | BLOCK origin=[2.5,70.5,4.5], block_state resolves `minecraft:stone`, TOOL present as empty instance; block_entity/radius absent. |
| Same block with explosion decay radius 4 | Radius=Some(4.0), never absent or zero substitution. |
| Broken named container | Saved block-entity Tag equals pre-removal metadata including custom name; subsequent removal does not alter snapshot. |
| FISHING roll with explicit stick and no entity | Origin equals supplied coordinates; TOOL item=`minecraft:stick`, count=1 and exact components; this absent. PR arismoko/horizon#2595 retains ownership of roll API semantics. |
| Deferred enchantment/trade/custom key registration | Exact `UnsupportedLootParameter`, `UnsupportedLootParameterSet`, or custom refusal; identical native/wasm result before evaluation. |
| Missing BLOCK state / false / Unit | Missing required state refuses before callback; future false bool returns Some(false); future trade token returns Some(()) versus None. |
| Malformed and excessive inputs | NaN/infinite origin, negative radius, oversized nested data refuse without callback or partial output; legal depth-512 data and largest vanilla fixture fit shared bounds. |
| Panic/trap; nested roll; no listeners | No partial candidate output; outer snapshot unchanged after nested scope; no-listener output and RNG sequence exactly match baseline with zero context conversions. |

## Proposed build stages

1. Verify exact vanilla required/allowed sets using REA initializer/lambda evidence; enumerate every gameplay producer of both evaluators. Reconcile the owner-supplied read list. File none in this docs-only change.
2. Build the named BLOCK/ENTITY/FISHING subset and typed capability registration, replace obsolete snapshot fields, and regenerate both targets and crossing coverage in one PR. Add boundary fixtures before broadening producers.
3. Implement deferred interaction/enchantment/trade projections only with verified producer routes and matching native/wasm fixtures. Design container and custom-key schemas separately before enabling registration.

## Open questions

- Which additional facts do the owner-supplied reads actually require? Recommendation: retain the initial subset until the owner-supplied read list names its parameters and types; add only API-owned snapshots justified by those reads.
- What read-only projection should CONTAINER expose? Recommendation: defer rather than grant mutable slot authority or pretend a live SlotProvider is a serializable value.
- Which shared byte budget accommodates maximum legal tool/block-entity snapshots? Recommendation: establish measured vanilla capacity in stage one and reuse/increase the shared budget; do not introduce a smaller loot-only limit.
