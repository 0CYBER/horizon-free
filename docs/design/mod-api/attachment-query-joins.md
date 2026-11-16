# Design: Attachment query joins

Provisional proposal for [arismoko/horizon#2497](https://github.com/arismoko/horizon/issues/2497), extending [arismoko/horizon#1108's attachment query contract](../../modding/src/concepts/attachments/querying.md). Docs only; approval precedes implementation. The existing single-type API remains the same operation, extended through typed query specifications, without aliases or compatibility paths. Follow [Part instance state](part-instance-state.md) for lifecycle identity and root commit rules.

## Evidence and route owners

Vanilla 26.3 `BlockEntity.saveCustomOnly(HolderLookup.Provider)`, inspected using `tools/rea_vanilla.py` from `/workspace/horizon`, delegates object-owned saving through ValueOutput. It establishes no attachment join contract. Vanilla has no counterpart to this mod query facility. The issue explicitly names no NeoForge counterpart: this proposal is judged on its merits. NeoForge 26.3 query behavior is unverified; no public documentation establishing such a query was inspected and no NeoForge source or patches were used. Public documentation would be required before adopting any claimed NeoForge contract.

Verified current owners (repository-relative paths):

| Route | Owner and consequence |
| --- | --- |
| Single-type capture and write validation | `crates/horizon-client/src/game/modding/attachment_index/mod.rs:31` and `:43`: select `self.dim`, lock its index and call `snapshot_named`; writes validate only that current index today. Cross-dimension routing must replace that limitation for explicitly scoped rows. |
| Capture callee | `crates/horizon-mod-runtime/src/attachments/index.rs:241`: hash lookup and Arc clone, no holder scan; `:257` validates residency with an owner hash lookup. |
| Borrowed traversal | `crates/horizon-mod-runtime/src/attachments/query.rs:96`: read each captured entry through `read_validated`; decoding can allocate and fail. No default insertion. |
| Decode callee | `crates/horizon-mod-runtime/src/attachments.rs:497`: query read/cache path, distinct from canonical writes. Decode cost depends on codec and bytes, not constant time. |
| Residency installation and retirement | `crates/horizon-mod-runtime/src/attachments/resident.rs:26` and `:42`; index `:193` registers, `:344` unregisters, visiting that owner's keys. |
| Saved entities, dimensions, players | `crates/horizon-client/src/game/modding/attachment_index/mod.rs:55`, `:80`, and `players.rs:7`: bind at lifecycle boundaries, never during query reads. |
| Journaling and nesting | `crates/horizon-client/src/game/runtime/transactions.rs:191`, `:237`, `:302`: open, merge/rollback and capture attachment maps. This is a holder-map journal, not a need for simultaneous mutable entry references. |
| Native/wasm boundary | `crates/horizon-mod-vocab/src/entity/attachment_query.rs:63` and `:80`: existing manual API-owned Wire token/batch conversion; evidence of the current route only, not a serialization pattern to extend. New crossing types must derive Wire. |

These owners establish existing mechanics, not implementation of the contracts below. Before code, audit every arismoko/horizon#1606 residency producer, lifted-pass exit, world close, transfer, chunk unload and save gate against the same publication boundary; the exact arismoko/horizon#1606 owner integration remains a prerequisite.

## Typed API and atomic writes

Choose tuples over a builder for access declarations, with options for scope/filter. Illustrative proposed syntax:

```rust,ignore
host.query((read(&MANA), read(&TEMPERATURE)), options)?;
host.query_mut((write(&MANA), read(&TEMPERATURE)), options)?;
host.query_mut((write(&MANA), write(&TEMPERATURE)), options)?;
row.apply(host, (Set(8), Keep))?;
row.apply(host, (Set(8), Remove))?;
```

Support arities 1 through 8 through generated tuple implementations. Reads require Send + Sync + 'static; writable slots additionally require Clone, as today. Read slots expose borrowed captured values and cannot accept replacement operations. No escaping mutable references. Reject duplicate keys, including two read slots, before capture with proposed `DuplicateQueryKey`; reject empty/oversized tuples as `InvalidQueryArity`. Declaration/type incompatibility remains an error, never a skipped holder. A row is a holder having every requested key, not a Cartesian product; joins never cross holder identities or dimensions.

`apply` opens one nested savepoint, validates every captured holder/residency and expected binding revision, pre-encodes all replacements, then invokes ordinary journaled writes for each writable slot. Merge only after every operation succeeds; otherwise abort the entire row. Recommend proposed `StaleBinding` when a queried binding has changed since capture, including remove/reinsert. This adds optimistic conflict checking for joined writes; existing single-key semantics need not change. Snapshot values never change after apply. A read slot's revision is also checked because the decision may depend on it. Same-holder/key recursive edits refuse; no borrow or index lock crosses a codec or callback.

Two writes to one holder reuse its first journaled map image. Different dimensions must journal under scoped dimension identity, never the currently selected relative Holder alone. Nested commit merges into the caller's action; root abort restores both types, membership, dirty flags and queued sync. There is no transaction on batch drop and no automatic transaction around a whole loop: use `host.transaction` for all rows. Root commit alone publishes journal emissions and retirement during lifted passes; save refuses while an action savepoint is open. Rollback must not invoke mod code.

## Scope and residency

Options select `Current`, `Dimensions([DimensionRef, ...])`, or `AllLoaded`. Current is the default. Explicit references validate world and loaded status, deduplicate, and refuse unloaded dimensions without loading them. AllLoaded captures the loaded-dimension registry at acquisition, including parked dimensions, never persisted unloaded worlds. A newly loaded dimension appears only in a fresh batch.

Capture all selected per-dimension versions on the owning server thread in one non-reentrant capture phase, without callbacks, ticking or dimension swaps between captures. This is one logical snapshot, not sequential observations across ticks. Each row carries its captured owning dimension for validation/routing; entity identity still uses the existing lifecycle epoch/id and contains no dimension. Reuse existing holder lifecycles and residency tokens; add no second generation.

Parked residents remain visible. Lifted residents remain visible exactly once through arismoko/horizon#1606's authoritative owner table and stable attachment binding, despite temporary absence from a raw list. Never enumerate raw lists to reconstruct a query. Queued removal remains a residency view until the committed removal boundary. Transfer publishes departure/arrival atomically at root commit; a fresh capture cannot see both copies. Old captured values remain readable; writes after residency changes refuse rather than retargeting a successor. If a lifted owner cannot safely accept a write, refuse explicitly before mutation; never treat temporary list absence as permission to recreate it.

Cross-dimension apply resolves the captured dimension through the host's world service under one transaction; it must not silently change the host's selected dimension or run gameplay callbacks under another dimension. Client reads cover only loaded authorized replicas; writes require ServerHost. World close invalidates live authority, without changing already captured immutable values.

## Holder filters

Use API-owned `AttachmentHolderKind::{Entity, Player, BlockEntity, Chunk, Level}` plus optional API registry handles for entity type or block-entity type. Players are a distinct holder variant; Entity does not implicitly include Player. Validate filter/type compatibility and registry handles before capture. Unknown or foreign registry handles refuse, not an empty accidental match.

Store immutable kind and resolved API type id with residency metadata. Check the candidate row's metadata before probing the other types or decoding values. This is O(1) per candidate, not zero total work and not a separate kind index. No engine enums cross the API. Replacement refreshes metadata through the existing lifecycle owner; type-preserving mutations retain it. Plain blocks and item stacks remain outside supported attachment holder kinds.

## Cost, transport and untrusted input

For k keys in D selected dimensions, let n_d be the smallest bucket size in dimension d and R the matched rows. Capture costs expected O(D*k); matching costs expected O(sum(n_d*k)), plus O(R*k) value access and actual codec work. With bounded k this is O(sum n_d), not O(all holders). Absent keys make that dimension empty immediately. Explicit dimension validation costs O(D); AllLoaded necessarily enumerates D loaded dimensions only when called.

The current dense bucket snapshots lack immutable per-key membership probes. Add a hash map from existing scoped holder/residency identity to row slot in each versioned bucket; capture its version together with dense rows. Do not build it by scanning the larger bucket at query acquisition. Probe those captured maps, never live mutable maps, to preserve snapshot joins. Retained versions require O(bucket size) metadata/map copy on first subsequent mutation; ordinary hash operations are expected, not worst-case, O(1). Memory is O(total indexed memberships) plus retained versions and O(D*k) batch descriptors. Decode/encode and transport add costs proportional to visited payload bytes. Native warm traversal allocates no per-row metadata; first decode may allocate. Wasm whole-batch serialization materializes matched rows and costs O(R*k + bytes); do not claim O(1) acquisition for wasm.

No query or attachment/lifecycle mutation means zero extra index work per tick: at most an O(1) enabled-path branch, zero allocations, codecs, holder scans or all-mod scans. Unbound holders create no query memberships. Lifecycle work touches only that holder's attachment keys. Subscription/type keys resolve at registration. Benchmark against arismoko/horizon#1811's 10,000-holder baseline, reporting capture, traversal, codec, transport, retained-version copy time and allocations separately; no invented timings.

Native and wasm share API-owned Wire query specs, scopes, filters, row errors and scoped apply payloads through the existing generator. One whole-batch capture import; one whole-batch apply import for submitted updates, never per-row imports. Validate the complete apply payload before executing row savepoints; an explicit enclosing transaction supplies whole-batch atomicity. Every new public crossing type must carry an explicit `#[derive(Wire)]` (fully qualified as `#[derive(horizon_mod_api_macros::Wire)]` where needed). The generator builds only the host call tables; serialization belongs to the derive, never copied or extended manual token/batch Wire implementations. The derive entry point is `crates/horizon-mod-api-macros/src/lib.rs:6`; existing derived public types include `crates/horizon-mod-vocab/src/state_predicate.rs:6`. Require the shared Wire capability for every boundary type and refuse registration with proposed `UnsupportedQueryWireCapability`, naming type and capability. No hand-written codec or guest pointer crosses the ABI.

Treat keys, scopes, tokens, lengths and replacements as untrusted. Proposed limits: 8 keys, 256 explicitly named dimensions, the existing shared NBT limits (64 MiB encoded bytes and depth 512), and existing shared Wire message budgets; refuse oversize before mutation, never truncate or constrain vanilla storage capacity. AllLoaded over transport budget returns a capacity error, never partial silent results. Query reads preserve malformed saved bytes and report row errors without defaults. Native unwind or wasm trap aborts active savepoints and pending publications under runtime callback policy; process-aborting panics are unrecoverable. Disable/report the failing callback according to runtime policy. Tuple and explicit-dimension limits require approval before implementation; payload limits reuse the shared boundary budgets. The current batch conversion calls `Limits::default()` for both encoding and decoding (`crates/horizon-mod-vocab/src/entity/attachment_query.rs:93` and `:108`); its defaults are defined at `crates/horizon-core/src/nbt.rs:202` and `:204`. Wire nesting has its own representation-aware depth budget, 1040 (`crates/horizon-mod-vocab/src/wire/mod.rs:73`), not an NBT depth override.

## Acceptance matrix

Expected future tests, not results from this documentation change:

| Fixture | Concrete expected result |
| --- | --- |
| A has mana 7/temp 20; B only mana 9; C only temp 30 | One joined row A=(7,20); zero defaults, dirty flags or packets |
| A write/read join | Set mana 8 succeeds; temp remains 20; old batch remains (7,20) |
| A write/write; second codec fails | A remains (7,20), zero dirty/sync publication |
| Set A=(8,21), nested commit, root abort | A=(7,20); zero outbound updates |
| Captured A; live temp becomes 22 | Apply refuses StaleBinding; mana remains 7, temp 22 |
| X contains A=(7,20), parked Y contains B=(9,30) | Current X yields 1 row; AllLoaded yields 2; sum mana=16 |
| Lift B out of Y list during pass | AllLoaded still yields B exactly once; no extra membership |
| Transfer B Y to X; root abort | B remains in Y once; zero transfer publication |
| Unload Y after capture | Old B reads (9,30); apply refuses StaleHolder; fresh AllLoaded has 1 row |
| Zombie, cow, player, chunk each have both keys | Entity/zombie yields 1; Entity yields 2; Player yields 1; Chunk yields 1 |
| Same coordinates in X and Y | Write X to 8 leaves Y at 9; foreign-world token refuses |
| Two keys with sizes 10,000 and 100, overlap 40 | 100 candidate visits, at most 100 other-key probes, 40 rows; never 10,000 candidates |
| 10,000 unbound holders, 100 idle ticks | Zero new memberships, codecs, allocations and index work |
| Duplicate key; 9 keys; 257 explicit dimensions | Refusal before capture; zero writes |
| Valid 65 KiB replacement within shared budgets | Succeeds; no join-specific 64 KiB refusal |
| Encoded NBT over 64 MiB or nesting beyond shared depth 512 | Refusal before mutation; A remains (7,20), zero payloads |
| Native and wasm fixture A/B/C above | Both return A=(7,20), identical errors; zero per-row imports |
| Missing Wire capability | Registration names failing type/capability before first query |

## Proposed stages

File: none. First approve tuple access, atomic row apply, scope/filter and limits. Then audit arismoko/horizon#1606 lifted and parked ownership, retirement, cross-dimension journal keys and save refusal. Add snapshot-compatible probe maps and residency metadata; require explicit `#[derive(Wire)]` on every new public crossing type, with no copied or extended manual Wire implementations; extend generated native/wasm host call tables for capture/apply together. Finally implement the matrix and arismoko/horizon#1811 comparative benchmark before advertising support.

## Open questions

- Resource budgets: reuse existing shared NBT and Wire budgets; approve only the tuple and explicit-dimension limits above; maintainer approval is required before code.
- arismoko/horizon#1606 integration: recommend stable bindings through lifted passes and root-only publication; name and verify every concrete owner and save-gate route in the implementation PR. The current route audit does not establish those end-to-end guarantees.
