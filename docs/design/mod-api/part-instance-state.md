# Design: Part instance state

**Accepted with changes** as a provisional design by the [maintainer decision](https://github.com/arismoko/horizon/pull/2328#issuecomment-6729846956). The requirements below must be satisfied before code. Attachments are owned by the world-visibility lead; that lead replaces the earlier attachment owner.

Design for [arismoko/horizon#796](https://github.com/arismoko/horizon/issues/796), jointly defining the state binding needed by [arismoko/horizon#781](https://github.com/arismoko/horizon/issues/781). This is not an implemented guarantee. The acceptance is conditional on the requirements below.

## Evidence and limits

Vanilla 26.3 was inspected from `/workspace/horizon` using `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.entity.BlockEntity saveCustomOnly --overload 0` and `python3 tools/rea_vanilla.py method net.minecraft.world.item.ItemStack copy`. The former builds a compound through ValueOutput and delegates custom saving; the latter creates a new stack with copied components and count (empty copies return EMPTY). These verify object-owned persistence and stack value copying, not a general retained Part identity contract. Vanilla has no Horizon Parts. General entity lifetime, block replacement and network ordering were not verified against vanilla in this investigation; the rules below are proposals, not vanilla parity claims.

The public [NeoForge attachment documentation](https://docs.neoforged.net/docs/datastorage/attachments/) describes holder defaults, optional serialization and client synchronization, but the retrieved page is labelled **26.1**. It also directs item stack data to components. The [26.3 AttachmentType Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/attachment/AttachmentType.html) was inaccessible through the web reader. Consequently **NeoForge 26.3 behavior is unverified**, including defaults, serialization, synchronization, replacement/respawn copy policy, retained identity and rollback. Class-level coverage summaries cannot establish these contracts. No NeoForge counterpart is claimed for general Part identity; public 26.3 AttachmentType and holder lifecycle documentation are required before claiming equivalence. arismoko/horizon#796 names no NeoForge counterpart; arismoko/horizon#781 names data attachments. No NeoForge source, patches or other project's source were read. Rechecking public 26.3 documentation is an implementation prerequisite.

## Identity versus storage

Retain scoped, non-owning handles; translate to relative storage keys only after validating the handle against its owning world and incarnation, and dimension for dimension-scoped holders. Never interpret a saved reference in the Host's currently selected dimension. A relative `Holder` is an internal address for an already validated operation, not a durable object identity. Retaining a handle neither loads nor pins a holder. World close invalidates all its handles. Epochs and generations must not wrap into valid old identities; refuse allocation on exhaustion.

Use the existing attachment codec/store as the only Part-data store. A Part object is shared behavior, not the owner of an instance's mutable data. Registry keys identify declarations, never instances. Persist declared values under namespaced attachment keys; do not serialize runtime handles as durable references. Mods needing durable links explicitly store a world/dimension plus persistent identifier or position and resolve afresh, accepting absence and replacement.

## Inspected owners and holder kinds

Paths and lines are relative to this repository. These are current owners, not proof of end-to-end guarantees.

| Kind | Verified owner | Proposed binding |
| --- | --- | --- |
| Block | `crates/horizon-mod-runtime/src/attachments.rs:66` defines relative holders and has no block variant | A plain block has no independent attachment holder today. Plain-block chunk entries are deferred outside the first implementation stages. A separate decision must establish the block-write lifecycle owner and incarnation-preserving mutations before adding position/generation storage. |
| Block entity | `crates/horizon-mod-vocab/src/lib.rs:3784` defines BlockEntityRef; `crates/horizon-client/src/game/runtime/transactions.rs:68,86` captures generation | Bind attachments to (position, generation) within the scoped level. Same-position replacement creates a new generation even with the same type. Property changes preserving the instance keep its data. |
| Entity, including player | `crates/horizon-mod-vocab/src/lib.rs:1610` defines EntityRef; `crates/horizon-mod-vocab/src/entity/attachment_holder.rs:13` wraps entity/player handles | Entity handles must carry no dimension: validate world and lifecycle epoch with id, resolving the current level through the entity table. Each dimension has one attachment index. Transfer is per kind: a lone mob or general row is rebuilt under a new id, so the old handle is stale and the state arrives under the new id; a vehicle a player rides keeps its id and handle. The old dimension's index keeps its rows until the root commit and the arrival rebinds. Respawn is a new incarnation; copy only according to the declared copy policy. |
| Item | `crates/horizon-mod-runtime/src/attachments.rs:66` has no item holder; vanilla ItemStack.copy above owns component copying | Item state belongs to the stack value through components, not an invented attachment holder. A retained slot reference identifies a versioned location, not a stack traveling through inventories. Copy/split/merge policy must be explicit; recommend copy value on split and merge only equal state. |

`crates/horizon-mod-vocab/src/entity/attachment_holder.rs:13` also includes chunk and level holders: scope them to residency/world epochs. They can own backing storage without becoming a second Part-state abstraction. Block and item support are proposed extensions, not claims of existing API support.

## Typed authoring and object safety (arismoko/horizon#781)

Require a typed authoring declaration with associated Data and a typed attachment declaration, erased once at registration into the existing codec/store and object-safe dispatch. Do not add associated types or constants directly to the trait currently stored as `dyn BlockEntityPart` (`crates/horizon-mod-vocab/src/parts/block_entities.rs:37`). Generate the adapter rather than duplicating native and wasm paths. Reject duplicate keys with incompatible schemas at registration. Give each hook its existing Host; obtain state through the holder's methods, never a second Host side table.

Before the first code PR, specify the minimal typed Data declaration signature and bounds, attachment key and generated erased adapter. Extend existing `AttachmentHolder` vocabulary rather than adding parallel Part identities. Show holder reads taking `&dyn Host` and writes taking `&mut dyn ServerHost`, with explicit missing-holder, stale-handle and schema-error results.

Persistence codec support and ABI support are separate capabilities: serialization does not establish `Wire` compatibility. Declare each state type's required shared Wire capability for every native/wasm boundary crossing. Unsupported capabilities must refuse registration/load with the capability and type named, before first access. Add generated coverage rows for this surface on both targets.

The first code PR must explicitly define edit reentrancy: same-holder/key recursive edits must refuse, and replacement must check the revision captured before invoking the edit closure. No mutable borrow may cross dispatch.

Prefer snapshot reads and checked replacement/edit closures over escaping mutable references. Defaults are lazy, owned per holder, and participate in transactions. Read-only absence inspection must not insert defaults. An edit commits a new encoded value only after successful validation. No data borrow may survive a callback, transaction boundary or await. This note proposes answers for both issues' shared identity/erasure questions; it does not close either issue or settle the exact authoring syntax without review. arismoko/horizon#781 can implement the binding separately after the arismoko/horizon#796 lifetime contract is accepted.

## Persistence, residency and synchronization

Save only committed, codec-backed values. Codec-less values are session-only and must not request persistence or sync. Load into a fresh runtime incarnation; decode errors must not expose successfully initialized typed state; they preserve the saved bytes and report failure rather than silently overwriting them with a default. Unknown keys remain opaque, bounded saved data. A codec version mismatch requires explicit failure or an approved schema conversion, never an API compatibility shim.

Unload invalidates live access without implying logical deletion of saved data. Reload restores values but requires fresh handles. Replacement and destruction retire the old incarnation and prevent old packets or handles from addressing its successor. Follow [multi-block placement](multi-block-placement.md) and [arismoko/horizon#1606](https://github.com/arismoko/horizon/issues/1606): residency publication and attachment retirement occur at the existing residency boundary. No second raw entity-list mutation route. The chunk residency/unload owner must clear block-entity live attachment bindings when unloading the chunk, preserving committed saved values; implementation must name and audit that concrete owner. For block entities, defer retirement of (position, generation) attachments until root commit; rollback restores the old generation and its values, while leaked tentative handles remain invalid forever.

Current sync ownership is `crates/horizon-client/src/game/modding/attachment_sync.rs:34` (whole holder values) and `:158` (remote map). Reuse the shared typed payload route requested by [arismoko/horizon#813](https://github.com/arismoko/horizon/issues/813), not a Part-specific packet protocol. Propose an envelope containing scoped holder incarnation, attachment key, schema and monotonic committed revision. Entity wire identity must use (lifecycle epoch, id) or the entity network id, never a residency join: joins are engine-internal per-list values, unstable across save/load and unsuitable as network identity. Send a full snapshot when tracking begins, then committed updates/removals to authorized trackers only. Reject stale incarnation/revision messages, bound snapshot buffering, and request a fresh snapshot on gaps. Client replicas cannot mutate authoritative state. Visibility loss clears replicas; rejoin receives a full snapshot. This is a required future contract, not proof that today's transport already provides it.

## Transactions, trust and cost

Extend the existing root transaction coordinator (`crates/horizon-client/src/game/runtime/transactions.rs:68,86,548`), which already captures/restores block-entity generations. Entities spawned or removed inside an action follow the same deferral: no attachment retirement before root commit; abort restores the pre-action bindings and invalidates tentative handles. Journal defaults, edits, retirement, dirty flags and pending sync together. Nested commit merges into its parent; nested abort restores its entry state; root abort publishes zero payloads. Defer sync and irreversible effects until root commit. Save while tentative state exists must refuse through the same save gate proposed by the placement design. Revalidate holder generation and read versions at apply; stale decisions refuse rather than recomputing against another object.

Native unwinding and wasm traps abort the active action and discard tentative state and messages; report and disable the failing callback according to runtime policy. Process-aborting native panics cannot be recovered. Restoration must not call mod codecs or callbacks: capture rollback material before invoking untrusted code. Encode/decode failures also abort without partial publication.

Treat mod input as untrusted on both targets: validate scope, side, ownership, keys, schema, numeric lengths, nesting, finite numeric fields where required and payload budgets before mutation. Recommend a declared 64 KiB encoded value limit and depth 64, with oversize refusal rather than truncation; exact limits need approval. These limits apply only to this new mod-state facility; they must not constrain vanilla stack components or replace shared Wire budgets. Arbitrary domain values remain the declaration's responsibility. Wasm uses opaque validated handles and the same schema/policy and transaction rules; no guest pointers or Rust Any values cross the ABI. Native has no exemption from validation.

Resolve declarations and subscriptions at registration. With no state declaration or listener, allocate no per-holder Part data, encode nothing, send no packets and perform no all-mod/per-tick scan. Existing handle validation and transaction guarantees remain applicable. Cost with subscribers is proportional to touched values and authorized recipients.

## Acceptance matrix

These are proposed implementation tests with hand-written expectations, not tests run for this note.

| Fixture | Expected values |
| --- | --- |
| Holders A/B, default counter 0; set A=7 | A=7, B=0; distinct default instances; Part object owns neither value |
| Same position in dimensions X/Y | Set X=7 leaves Y=0; X handle used with Y Host refuses |
| Remove generation 4; recreate generation 5 | Old handle MissingHolder; new counter 0; late generation-4 update ignored |
| Save A=7, unload and reload | Fresh handle reads 7; old handle refuses; codec-less counter resets to 0 |
| Outer starts A=7; inner sets 9 and commits; outer aborts | A=7; zero outbound updates and no committed dirty flag |
| Inner inserts default then panics/traps | No stored default after abort; subsequent valid edit to 8 succeeds |
| Placement replaces block entity then cancels | Original generation and counter 7 restored; tentative handle refuses |
| Tracking joins at revision 10, update 11 then stale 10 | Client remains at revision 11 value 8; visibility loss removes replica |
| Entity transfers X to Y | A lone mob or general row: old handle `Stale`, the value under its new id in Y. A vehicle a player rides: the same handle resolves Y. Old dimension index retires at the root commit and arrival rebinds; no dimension in handle. Inside an action rolled back after the change of level, the state stays with the entity under its new id and the old handle stays `Stale` |
| Entity spawn/removal inside action then abort | Original bindings restored, tentative handles invalid, zero retirement/publication before commit |
| Chunk unload with block-entity state 7 | Chunk residency owner clears live bindings; committed saved 7 survives |
| Internal residency join changes across reload | No join in wire envelope; stable entity identity and revision govern acceptance |
| Same-holder/key recursive edit | Explicit refusal; outer replacement checks captured revision; no escaping mutable borrow |
| Unsupported Wire capability or undecodable schema | Registration/load names unsupported type/capability; saved bytes preserved and no initialized typed value exposed |
| Stack count 4, state 7, split 1 | Counts 3/1, both state 7; unequal state 8 cannot merge |
| Bad scope/schema or 65 KiB value under proposed limit | Refusal, old value 7 preserved, zero payloads |
| No subscribers/declarations, 100 ticks | Zero Part-state allocations, codec calls and packets |

## Proposed implementation stages

File: none. These are work packages, not changes made by this note.

1. Approve identity, unload and item/block decisions; verify remaining vanilla routes and public NeoForge 26.3 evidence.
2. Audit every handle-to-relative-holder conversion, residency owner and generation retirement path; test replacement, world close and dimension transfer.
3. The first code PR must explicitly specify typed state registration (minimal signature, bounds, key and erased adapter), the shared Wire capability required by each state type, and edit reentrancy/revision rules. Implement arismoko/horizon#781 on existing attachment storage with native/wasm generated coverage and explicit capability/type registration/load failures.
4. Integrate save/load and nested rollback, defaults and panic restoration; plain-block chunk entries remain deferred to a separate lifecycle-owner decision.
5. Complete arismoko/horizon#813 typed payload integration and tracker snapshot/revision rules; test stale messages, unauthorized recipients and both targets.
6. Demonstrate the acceptance matrix through real owners and measure the zero-subscriber path before advertising support.

This unblocks arismoko/horizon#781's per-holder Data binding, arismoko/horizon#813's shared synchronization, and arismoko/horizon#792's general entity state/lifecycle work. It supplies the state lifetime prerequisite for arismoko/horizon#855 placement rollback and arismoko/horizon#1606 residency integration; it does not implement or replace those issues.

## Open questions

- **Deferred plain blocks:** decide the block-write lifecycle owner and incarnation-preserving mutation set separately; plain-block chunk entries are outside the first stages.
- **Attachment integration:** the world-visibility lead must confirm entity epoch/id identity and wire identity against the attachment index and sync code before implementation, replacing the earlier request for ecs-lead confirmation. Name the concrete chunk-unload owner that clears live block-entity bindings.
- **Item identity:** recommend stack value components and versioned slot access, not stable traveling item identities. Approve split/merge/copy semantics before exposing ItemPart Data.
- **Authoring syntax:** recommend a separate typed declaration erased into object-safe Part dispatch; the first code PR must specify whether DATA is an associated declaration or registration argument, its bounds and adapter, required Wire capabilities, and edit reentrancy rules. Do not break dyn storage accidentally.
- **Unload continuity:** recommend fresh handles after reload, with saved values preserved. Persistent references must resolve explicitly rather than resurrect handles.
- **Transport limits and authority:** recommend 64 KiB/depth 64 and server-authoritative replicas; confirm limits and revision-gap recovery with arismoko/horizon#813's owner.
- **Parity evidence:** recommend accepting these as Horizon proposals while requiring public 26.3 verification before claiming NeoForge equivalence. Vanilla entity/removal/network routes also need clean-room inspection during implementation.
