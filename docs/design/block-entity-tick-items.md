# Dropped-item writes during mod block-entity ticks

This design implements the port's transactional requirement tracked in arismoko/horizon#2846, not vanilla or NeoForge behavior. NeoForge adds no event, deferral or drop capture around a block entity's tick; newly spawned item entities join immediately. Its block-break and entity death/shear captures are separate mechanisms. This overlay exists only to keep existing-item host writes atomic with acceptance of the port's mod block-entity snapshot.

With no scope open, the engine behaves exactly as vanilla for these item operations: changes are immediate, the overlay remains None, and nothing is staged. New item spawning remains immediate even while the mod tick scope is open.

## Acceptance

An engine-owned scope on Game begins before the mod tick callback and ends only after put_back_mod_block_entity. Its item overlay starts as None. The first dropped-item write captures the residency owner and join incarnation, the complete original stack, and the proposed replacement or removal. Numeric IDs alone never identify a pending write. Subsequent writes replace the proposal. Scoped reads observe proposals, and a proposed removal hides the entity from resolution, narrowing, selectors and item-place reads.

The final block-entity snapshot is prepared and validated without publication. Acceptance checks its revision and stack conversions, the block-entity table generation and the placeholder ownership, and every captured item precondition. Only then are block-entity fields and all proposals published without intervening mod callbacks. The overlay is detached before deferred removal notifications run. A stale precondition, missing or replaced block entity, invalid snapshot, unbalanced explicit transaction, or malformed wasm response rejects the whole proposal. Decode failure poisons the scope even if earlier host calls succeeded. Existing revision invalidation is retained. Rejection never replays work next tick.

Native and wasm use the existing host calls and Wire types. No public batch method or generated host row is added. Explicit nested host transactions save the overlay proposals: abort restores the savepoint, and commit retains proposals in the tick scope without publishing them.

## Cost

The no-write scope stores fixed metadata and None, without cloning stacks, allocating collections, or scanning mods. Root's selected implementation keeps the overlay on Game. Only item-entity read, write, resolve and select paths check its Option; no separate facade or generator change is required. The PR lists every function that gains this branch. Measurements compare both the existing 1,000-block-entity tick benchmark with a trivial attached mod and plain item-entity reads outside any scope, before and after.

## Deliberately immediate operations

This is not a whole-world transaction. Pickup delay and pickup target, non-item removals, new item spawning, block changes, entity motion and teleportation, players, attachments, neighboring block entities, weather and world effects retain their existing semantics. An immediate operation that invalidates a pending item's precondition rejects the proposal but is not undone.

The rejected undo-journal alternative cannot provide this contract. Restoring an immediately changed stack after a reentrant merge or pickup can duplicate transferred items; conditionally refusing restoration leaks part of the rejected batch. An overlay avoids both outcomes by withholding publication.

## Regression map

Real adapter tests run tick_mod_block_entity through api_block_entity_tick, finish_block_entity_writeback and put_back_mod_block_entity, with prepared resources. Start with dropped counts 5 and 3, an empty block-entity slot, and proposals leaving 2, removing the second item, and inserting 6 into the block entity.

| Case | Independent expected outcome |
| --- | --- |
| Valid tick | Block entity contains 6, first item contains 2, second is absent; one removal notification; next no-write tick changes nothing. |
| Bad revision, actual invalid stack conversion, malformed wasm output | Original block-entity fields and item counts 5 and 3 remain; next tick cannot replay proposals. |
| Block entity replaced or removed | Replacement or removal remains effective; counts remain 5 and 3; no proposed fields reach the replacement. |
| Read after write, then later Part removes the same item | First read sees 2; subsequent resolution, narrowing, selectors and item-place reads hide the removed item; acceptance removes it once. |
| Second item's original stack independently changed to 4 | Reject the proposed block entity and first-item reduction; first remains 5, second remains 4. |
| Reused ID, changed join, or other level | Never mutate replacement or other-level entity; reject the whole pending proposal. |
| Two Parts | Later Part sees earlier proposals in registration order; final proposals share acceptance. |
| Nested transaction abort | Restore the outer proposals, not world storage; nested commit still waits for tick acceptance. |
| Removal notification reenters | Overlay is already detached; no replay or append to the accepted batch. |
| Disabled or stopped tick | No scope writes; unrelated block-entity dispatch stays unchanged. |
| No active scope | Vanilla immediate behavior: overlay remains None and ordinary reads and writes stage nothing. The adapter regression checks both overlay and key are None after its outside-scope setter. |

The real adapter regression `native_tick_publishes_or_rejects_item_proposals_with_writeback` covers native and real shared-memory wasm host writes, valid and malformed guest replies, actual invalid final stack conversion, typed setter refusal, registration order, nested abort, later removal, block removal, removal notification reentrancy, scheduler-disabled and sleeping ticks, unrelated dispatch, and immediate writes outside a scope. The wasm guest invokes the production setter and discard rows before its response is decoded; its failure is not a Rust fake. Guest identity fields are read-only on application, so a forged identity is ignored rather than treated as a changed authoritative identity.

The publication-layer regression `finalization_never_retargets_stale_items_or_replaced_placeholders` uses real Game storage and injects interference between proposal submission and finalization. It covers independent second-stack mutation to 4, a fresh join under the same numeric ID, an actual transfer to another level, replacement of the block entity including an empty-ID replacement, wrong snapshot identity, and abandoned explicit transactions. These tests exercise publication and putback directly; they do not claim to dispatch a Part for the interference step.

Each layer's tests kill its own mutants. `tools/test_block_entity_tick_items_mutants.py` compiles temporary immediate-publication, omitted-original-stack-check, and missing-decode-poison mutations, requires the corresponding behavioral assertion to fail, and restores the source after each. The previous api_values rejection fixture tests identity, not an invalid stack; the adapter regression supplies an actual count-65 plain stone stack whose engine conversion refuses it.
