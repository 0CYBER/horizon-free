# Design: chunk attachment disk hooks

Provisional proposal for [arismoko/horizon#857](https://github.com/arismoko/horizon/issues/857); no implementation or adopted guarantee. This answers the worker Host versus server-thread snapshot question: retain the existing server-thread capture seams, but execute mod load, save and persistence codec callbacks on a chunk worker with a restricted Host. Do not pass Game to a worker or silently relocate the requested callbacks to the server thread.

## Evidence and limits

Vanilla 26.3 was inspected from `/workspace/horizon` with `python3 tools/rea_vanilla.py method net.minecraft.world.level.chunk.storage.SerializableChunkData copyOf` and the same command for `parse`. `copyOf` rejects unserializable chunks, copies sections, light arrays and heightmaps, and collects block-entity tags and proto entities. `parse` constructs serialized chunk data and returns null when Status is absent. These establish separate capture and decoding boundaries, not a vanilla mod attachment hook or proof of worker scheduling. Vanilla callers, unload ordering and disk failure recovery remain unverified; audit them with the same oracle before code.

The issue names NeoForge 26.3 ChunkDataEvent.Load/Save. The public [attachment documentation](https://docs.neoforged.net/docs/datastorage/attachments/) describes optional persistence serializers but is labelled 26.1. The [26.3 event Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/event/level/ChunkDataEvent.html) was inaccessible. Therefore NeoForge 26.3 thread, event ordering, cancellation, exception, size and data-loss limits are **unverified**. Require public 26.3 event and attachment lifecycle documentation before adopting a parity contract. No NeoForge source, patches or other project's source were consulted. The rules below are Horizon recommendations pending that evidence.

## Verified current owners and route audit

Paths and lines refer to this checkout; these are inspected owners, not completed callback routes.

| Route | Current owner and required integration |
| --- | --- |
| Explicit full save | `crates/horizon-client/src/game.rs:14861` save_level captures full copies, calls the seam at :14888 and submits save_later; gate before capture and enqueue one worker callback per copy. |
| Streaming unload | `crates/horizon-client/src/game.rs:16089` handles proto snapshots; :16101 handles full copies from unloaded chunks before save_later. Hold recovery data until acknowledged persistence. |
| Late full generation | `crates/horizon-client/src/game.rs:14871` and :16080 call native_late_full_chunk_snapshot before save_finished. Use the same worker envelope, not a second hook. |
| Closing world | `crates/horizon-client/src/game/world/saving.rs:176` and :193 call proto/full seams. Drain callback and disk acknowledgements before destroying worker runtime. |
| Proto save, synchronous and asynchronous | `crates/horizon-client-render/src/streaming.rs:2534` prepares saves; :2578 finish_save selects synchronous save_protos or submit_protos; :1095 executes Job::Save. Both must pass the same callback boundary, including the synchronous route. |
| Lower-level bypass | `crates/horizon-client-render/src/streaming.rs:2519` save and :1570 save_finished can bypass Game dispatch. Centralize dispatch in the persistence job owner so these cannot omit it; avoid duplicate callbacks from Game seams. |
| Full/proto adapter | `crates/horizon-client/src/game/modding/native_chunk_save.rs:9,18,27` are server-thread preparation seams; :24 and :47 are empty. Retain capture authority here, replace empty dispatch with job preparation. |
| Disk attachment encode/decode | `crates/horizon-client/src/save.rs:1623` writes attachment NBT; :1885 restores it. Load callback belongs after this decode, before publishing the loaded holder; save callback precedes this encode. |
| Residency publication | `crates/horizon-client-render/src/streaming.rs:1530` installs loaded block entities. Require successful attachment load completion before any corresponding live publication. |

Forced saves, periodic saves, dimension transitions and shutdown must be traced to these owners during implementation; this audit does not claim their entire caller graphs are verified. Background job failure currently requires an explicit result/acknowledgement audit rather than assuming Done::Saved means durable success. Remote clients must never run authoritative disk hooks. Missing chunks and newly generated chunks with no decoded disk record do not fire load; proto-to-full promotion does not fire a second load.

## Proposed contract

Capture only committed attachment state on the server thread under the existing world visibility/save lease. An open action savepoint refuses saves through the arismoko/horizon#2211 save gate, including forced, unload and shutdown saves; do not serialize tentative state or take a second private transaction snapshot. Defer unload when the gate refuses. Reuse the existing lifecycle AttachmentHolder/ChunkRef and residency owner, scoped to world and dimension; do not introduce another chunk generation. A snapshot capability expires at callback return and never resolves back to the live world.

A job owns its isolated attachment store. Save dispatch runs after capture and before encoding. Listener writes affect that store only. Capture 7, listener writes 8, live world changes to 9: disk gets 8, live remains 9. Use immutable shared encoded values or deep owned copies; never share mutable typed values or hold a world lock across mod code. Dependency-ordered listeners run sequentially for a job. Serialize jobs for a chunk in capture order, reject stale completions after residency/world retirement, and never allow an older write to replace a newer successful save.

Load dispatch runs after bounded disk decoding and before publication. Its successful edits become the loaded attachment store; failure publishes no partially initialized holder. Full and proto disk loads follow the same rule. Unload notification follows successful save completion when persistence is required; unload itself does not delete saved attachments. Persistence does not imply network synchronization.

The worker Host implements shared Host facilities only where safe: immutable registry/settings snapshots, handle construction and a local transaction over this job's attachment store. It exposes no world mutation, scheduling into live state, live chunk lookup, Parts, packets or Game access. Ordinary ChunkRef live operations refuse in this context; snapshot attachment methods validate the job capability and existing holder identity. No mutable borrow escapes a callback. Recursive edits to the same key refuse, and callback failure aborts the whole job's tentative edits.

Native and wasm use the same catalog hooks, dependency order and generated adapter. Define API-owned Wire representations `ChunkDiskContextWire` (world scope, dimension, chunk position, full/proto kind and load/save phase), `AttachmentDiskEntryWire` (registered key, schema, bounded encoded value), and `ChunkDiskResultWire` (success or structured refusal). These are proposed names and fields, not existing types. Each declared typed value needs both its persistence codec and the shared Wire capability required by dispatch. Refuse registration before first callback if worker-safe execution, codec or Wire support is absent, naming the type and missing capability. No Rust Any, pointers or runtime handle serialization across wasm. A native module has the same validation obligations and must explicitly support concurrent worker invocation; unsupported modules refuse registration rather than gaining server-thread fallback.

## Failure, trust and cost

Propose 64 KiB per encoded value, depth 64 and 1 MiB total mod attachment data per chunk, including unknown keys; validate lengths before allocation, keys/schema/scope before mutation, and finite numeric fields where the schema requires them. Refuse oversize data without truncation. These budgets apply only to attachment data, not vanilla chunk contents. Preserve bounded unknown keys opaquely; do not turn unknown unrelated ids into new errors. Corrupt or over-budget input must remain recoverable on disk rather than being overwritten by defaults.

On codec error, native unwind or wasm trap, abort the snapshot transaction, report the mod/key/phase and keep the prior disk record. Disable the failing callback according to runtime policy; never silently retry without it and claim a successful save. A process-aborting native panic is unrecoverable. Rollback uses engine-owned captured bytes and invokes no mod code. Disk failure retains a retryable snapshot and dirty/recovery ownership, reports failure to wait/flush/close, and prevents silent successful unload. Partial publication and clearing dirty state before acknowledgement are forbidden. Exact atomic-write and crash durability guarantees require an audit of the disk writer before adoption.

Resolve subscriptions and capabilities at registration. With no listener, one cached subscription check selects the existing persistence route: zero new per-chunk allocations, codec calls, worker messages or mod scans. Existing attachment persistence costs remain. Subscriber work is bounded by the declared entries and budgets; use bounded queues and backpressure rather than unbounded retained snapshots.

## Acceptance matrix

Proposed tests; none are claimed executed for this design.

| Fixture | Concrete expected result |
| --- | --- |
| Capture 7, callback writes 8, live writes 9 | Reload 8; live 9; callback cannot read 9 |
| Load disk 8, callback increments | Published value 9; one load invocation |
| Full, proto, late, forced, unload, closing saves | Each captured record has exactly one save callback, before encode |
| Save revisions 10 then 11 complete out of order | Disk revision 11; revision 10 cannot overwrite it |
| Open action savepoint with tentative 9 | Save refused by shared gate; disk remains 7; zero callback invocations |
| Unload/reload | Saved 8 survives; old live handle refuses; fresh holder reads 8 |
| Codec error, unwind or trap after setting 8 | Disk remains 7; no partial load publication; structured failure |
| Disk write fails | Old disk 7 retained; snapshot 8 retained for recovery; unload/close reports failure |
| Native and wasm execute identical job | Both save 8; live lookup refuses; unsupported Wire registration names type/capability |
| Value 65537 bytes, depth 65 or total 1048577 bytes | Refusal; no truncation; prior record preserved |
| Zero subscribers, 100 chunk saves | Zero added allocations, callback calls and job messages |

## Proposed implementation stages

File: none. These are future work packages, not Rust changes in this note.

1. Obtain public NeoForge 26.3 evidence and approve worker capability, budgets and failure policy.
2. Complete vanilla caller and Horizon route audits, including forced/periodic/close and disk acknowledgements; integrate the existing save gate and lifecycle owner.
3. Specify and generate Wire declarations, registration refusals and native/wasm worker dispatch; centralize the callback boundary below all preparation routes.
4. Implement isolated load/save stores, bounded queues, rollback and ordered persistence acknowledgement without early dirty clearing.
5. Exercise the matrix through real disk routes on both targets and measure the unsubscribed path.

## Open questions

- **Thread contract:** recommend worker dispatch and server capture as above; require public 26.3 evidence before adoption, rather than infer scheduling from event names.
- **Lifecycle authority:** recommend the existing world-visibility owner approve snapshot capability validation and unload recovery; no second generation counter.
- **Failure handling:** recommend fail closed with retained recovery data and explicit close failure. Audit writer atomicity and decide crash recovery before advertising loss prevention.
- **Worker module safety:** recommend explicit registration capability for both targets; settle runtime instance concurrency and wasm fuel limits before implementation.
- **Budgets:** recommend 64 KiB/value, depth 64, 1 MiB/chunk plus bounded queues; confirm against the shared Wire budgets and unknown-key preservation policy.
- **Remaining routes:** recommend treating the unverified forced/periodic/transition caller graph and vanilla unload sequence as mandatory implementation evidence, not inferred coverage.
