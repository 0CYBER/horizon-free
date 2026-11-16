# Dropped items during block-entity ticks

During a mod's `BlockEntityPart::block_entity_tick`, existing dropped-item stack writes and removals are proposals. Use `ItemEntityRef::set_stack` to replace a complete nonempty stack and `EntityRef::discard` or `EntityRef::remove` to remove it. Item-place writes join the same scope. Native and wasm mods use the same calls; there is no separate batch method or transaction seal to finish the tick.

Later Parts see earlier proposed stacks in registration order. A proposed removal hides the item from handle resolution, item narrowing, entity selectors and item-place reads. Explicit nested host transactions can abort their own proposals; committing one waits for final tick acceptance.

The engine validates the final block-entity snapshot and every item's original stack and residency incarnation before publishing either side. Rejected revisions, invalid stack conversions, malformed guest responses, stale item preconditions, and removal or replacement of the ticking block entity discard the proposals. A successful host setter means the proposal was accepted into the scope, not that final writeback has already committed. A numeric entity ID never retargets a replacement or another level. Removal notifications run after publication and after detaching the completed overlay.

## Operations outside the transaction

This scope is not a whole-world transaction. Pickup delay and target changes, spawning, block changes, motion and teleportation, non-item removals, players, attachments, neighboring block entities, weather and world effects are immediate. They are not undone when the snapshot is rejected. An immediate operation that destroys a staged item's precondition can reject the pending proposal while retaining its own effects.

Outside a mod block-entity tick scope, these item writes and removals take effect immediately. Vanilla block-entity ticks also apply item changes immediately.
