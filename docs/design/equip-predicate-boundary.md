# Read-only equipment predicates inside menu loops

Implemented boundary for #2669 and #3549. The historical conversion analysis below
explains why the engine stack and API stack remain distinct owned types.

## Can we just borrow the existing stack?

**Not through the existing boundary.** The menus already borrow an engine stack,
not an API stack: `crates/horizon-client/src/menu.rs:615–636` and
`crates/horizon-client/src/lan/menu.rs:460–501`. Both belong to client-lib (whose
manifest uses these source paths). `Game::menu_ops` delegates through `menu_session`
(`crates/horizon-client/src/game.rs:9506–9585`); its `Env` lends only loop context
and player identities (`menu.rs:88–97`). `LoopContext` contains registry/settings
snapshots, not entity storage (`crates/horizon-world/src/native_load_context.rs:193–215`).

The engine stack owns `Arc<str>` and boxed extra data
(`crates/horizon-items/src/items.rs:417`); the API stack owns keys and an optional
`Arc<Components>` (`crates/horizon-mod-vocab/src/item/item_stack.rs:62`). They cannot
be reborrowed as the same type. The existing slot adapter converts before borrowing
(`game/modding/api_values.rs:1192–1208`, under the client source root). Even ordinary
Host-backed borrowed-stack hooks are not a zero-copy precedent: `native_use_item`
(`game/native_hooks/base.rs:3477`) calls generated dispatch whose crossing creates
an API value (`crates/horizon-mod-api-gen/src/dispatch.rs:909–942`). The generator
passes Game itself as Host, not a newly allocated Host
(`crates/horizon-mod-api-gen/src/host_param.rs:109–115`).

| Option | Engine/bridge allocations per candidate slot |
| --- | --- |
| Existing adapter, no subscribers | 0, but no readable target entity contract |
| Existing adapter, subscribed | Payload-dependent, not a fixed count: component-bearing stacks allocate an encoded compound and an API component owner, plus nested payload allocations; plain stacks may allocate 0 |
| Borrow the engine stack directly as API `&ItemStack` | Not type-correct; a cosmetic Host does not fix it |
| Prepare one API stack before slot traversal, then borrow it | 0 during traversal; preparation is payload-dependent and still needs a real Host/entity boundary |

Conversion evidence: `api_values.rs:711–728`,
`crates/horizon-items/src/item_data.rs:1110`, and API `set_components` at
`item/item_stack.rs:337`. Thus **hoisting conversion makes borrowing sufficient for
the stack cost**, without a new public stack representation, but does not remove
the need for the following small Host boundary.

## Proposed boundary and budget

Lend an engine-owned, call-scoped read-only Host adapter through both menu callbacks,
with an API-owned target entity handle and the prepared `&ItemStack`. Split borrows
of menu storage and entity reads; never put a borrowed Game behind the mutable menu.
Prepare the API stack once per distinct input stack version, before candidate traversal;
reuse it for all slots, not the predicate's answer. Read the actual engine stack's
resolved equippable component, including removals, slot overrides and absent versus
empty allowed-entity sets, before dispatch. Retain one `can_equip` moment and the
existing ItemPart/catalog dispatch, not a parallel loop-context variant.

The adapter exposes registry/tag/settings reads and target identity, type, position,
living state and equipment reads from a stable read view. These are observations,
not commands, RNG, scheduling, storage writes or a transaction capability. Variable
owned read results must be prepared outside traversal and borrowed during it; do not
promise arbitrary existing allocating Host methods are free. Unsupported reads report
unavailable explicitly, never fabricated values. Client prediction uses the local
player/replicated entity view at the click revision, without a server round trip;
the server uses its current authoritative target. Missing/stale handles refuse the
candidate, never alias a recycled entity id.

Decide captures stack/menu/entity revisions and prepares reads; fire asks once per
candidate slot; apply rechecks those revisions and slot preconditions in the same
transaction, refusing rather than recomputing on mismatch. Authoritative evaluation
still runs independently of prediction. Dispenser and pickup retain their candidate
iteration and other vanilla checks, using the same read adapter and stack data rule.

Maintainer ruling on the revision recheck: on the native path the synchronous read-only
borrow is the accepted equivalent of the recheck. The predicate is asked and its answer
applied under one immutable borrow, through a Host adapter that offers observations only
(no commands, storage writes, scheduling or random draws), so nothing can change in
between and a revision token would compare a value with itself. The recheck stays required
wherever deciding and applying are separated in time or state: (1) the prepared wasm
crossing (#3549), where a payload prepared once is reused across candidates: a stack
mutation invalidates it and a stale preparation refuses; (2) any path that keeps an answer
beyond the call (a cache, a predicted answer applied later). A stale or missing entity
handle refuses on every path.

Native lends the prepared values. Wasm uses the shared-memory Wire boundary: prepare
the stack and read-view payload once in guest storage, then send only call-scoped
handles, slot and scalar answer per candidate. No engine pointers cross; release
handles at decision end. This requires a reusable prepared Wire call path, not the
ordinary owned-stack encode/decode on every slot. Unsupported capabilities refuse at
registration/load with a named reason on both targets; no silent wasm skip.
The generator layer of #3549 implements prepared ownership per participating guest instance,
borrowed token-based asks, generation/release checks and load/registration refusal for missing
prepared exports. Slot and entity-kind names are prepared outside traversal and asks carry IDs.
Gameplay hoists preparation across candidates for one dispense, menu input scope and
one mob's pickup attempt. `EquipDecision` exclusively owns the source slot during
mutation; every mutable field access bumps its revision before lending, and
replacement/removal bumps its identity generation. Overflow retires the scope.
A changed-and-restored source remains stale. Menu reuse uses scope and scalar stamps,
not stack equality. Apply checks the source, menu and entity admission preconditions;
a recycled numeric entity id does not reuse an earlier admission. No-listener paths
retain their data answer without conversion or a new entity lookup.

The prepared target rows retain entity type (`EntityTypeKey`), position, health,
maximum health, alive/living status and the eight physical equipment slots. Native
lends these through an observation Host; wasm sends the rows once with the stack,
and serves these reads from guest storage. Equipment components remain shared API
owners rather than being converted or encoded again per read. `entity_kind` uses the
same allocation-free key API for every caller, with explicit `key.id(host)` name
resolution outside this budget. Other allocating Host reads remain ordinary calls;
this is not a promise that every Host operation is allocation-free. Registry/tag/settings
observations use scalar lookups; owned names and enumerations are outside the budget.

The required engine/bridge budget **per candidate** is 0 heap allocations, 0 stack or
component payload clones/copies, and only bounded scalar/handle copies, including
Host reads and guest dispatch. Setup is not free and must be measured separately;
stack mutations invalidate preparation. A counting allocator/no-allocation scope
around warmed traversal must cover both menus, multiple slots, component-rich stacks,
unrelated subscribed items, entity reads, and native/wasm dispatch. Check refusal
leaves the source and all other slots unchanged, stale revisions refuse, and data-only
stack overrides work. Mod-authored allocations are outside the bridge budget. Generator
loopback tests now cover retained stack identity, payload-size-independent scalar asks,
native/guest answers and token refusals; runtime tests cover memory-copy counts and
missing-export load refusal. Decision tests cover source mutation barriers, entity admission checks, gameplay apply
preconditions and the no-listener path. Native warmed traversal checks allocations;
wasm copy probes separate preparation bytes from payload-independent scalar asks.
