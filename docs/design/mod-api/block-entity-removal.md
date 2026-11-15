# Design: transactional block-entity removal

This is a contract for a future implementation, not an API available today. It covers removal of a
block entity through its owning block, not deletion from the block-entity storage map. It does not
specify a new method signature. The existing block and transaction paths need changes before this
contract can be offered.

## What the existing paths do

`Game::level_set_block` in `game/modding/native_entity_refs.rs` checks the level and position, then
calls `Game::native_set_block` in `game/native_hooks/`. The latter calls
`GameLevel::native_set_block` through `with_level`, then processes level events. The level method
calls `behaviours.set_block` and creates a new block entity when the landed block needs one. This
is the normal block lifecycle path; it is **not** currently made transactional by
`level_set_block`.

The separate `Game::journal_set_block` in `game/runtime/transactions.rs` is currently used by the native
fluid handler, not by that public block-write path. It keeps the first block state at each depth,
writes only the stored state, and queues `After::Block`. `transaction_close` merges a committed
child's first snapshots into its parent, retaining the parent's earlier snapshot, and appends the
child's queued work. `root_commit` restores the original stored state temporarily and calls
`level_set` for the final state; `roll_back` restores stored states without those updates.
`level_set` calls `behaviours.set_block` and processes level events. This fluid-oriented route is
not by itself a complete replacement for `GameLevel::native_set_block`: in particular the latter
also creates the appropriate new block entity.

`journal_block_entity` snapshots the entity, attachments, hopper state, custom name and extras
(including lock and loot state) on the first write at each depth. However, `roll_back` currently
restores the entity only if both the snapshot and a current entity exist. It cannot restore a
removed entity or remove a newly created one. `After::Changed` and `After::Updated` are also
position-based; `root_commit` coalesces `Changed` but does not generally coalesce `Updated`.
Neither direct deletion nor assuming the present journal already handles lifecycle changes is safe.

The vocabulary's `BlockEntityRef`, re-exported by `mod_api`, identifies a level, position and lifecycle generation.
`native_block_entity_target` rejects a replacement or reloaded entity at that position, just as it
rejects an absent entity or an old world epoch. In-place payload edits preserve identity. The
contract below preserves this identity rule; it does not offer transactional lifecycle writes yet.

## Removal contract

- Removal requires a valid, loaded block entity in the authoritative level in place. Stale-world,
  unloaded, missing and wrong-level handles are refused without changes. Validation must be at least
  as strict as `native_block_entity_write_key` and `native_level_write_allowed`.
- The operation changes the owning block through the normal block-write lifecycle. It must not leave
  a block that requires a block entity without one: removal while retaining such a block is refused.
  A same-position replacement that needs an entity creates the replacement through the block-write
  path. Whether an operation chooses air, a fluid legacy state, or another valid replacement is an
  explicit part of the future API contract, not an implicit storage deletion.
- The old container's contents, saved data, lock, name, attachments and capability-backed state belong
  to the old entity. On commit the normal block lifecycle determines drops and other removal effects;
  they must happen exactly once. They are not silently copied onto an unrelated replacement. A
  replacement's capabilities resolve from the replacement, not the old entity. In-place block state
  transitions that preserve an entity follow the existing block lifecycle instead of being treated
  as removal.
- At each nesting depth, reads and capability lookups must see the tentative result: after removal
  there is no old entity to access, and after replacement lookups address only the replacement.
  The implementation must retain enough old state to restore existence as well as data and
  attachments. It must not expose a partially removed container or leak old capability access.
- No drops, lifecycle notifications, client updates, comparator changes, menu closures or other
  externally visible removal effects may escape an abort. At root commit, publish the successful
  block transition and its normal effects in their lifecycle order, once. Existing `After` entries
  targeting an entity that was removed or replaced must not update the wrong entity; their ordering
  and invalidation must be implemented deliberately, not inferred from position alone.
- An inner abort restores the state at entry to that depth and discards its pending effects. An inner
  commit transfers its work to its parent; a later outer abort restores the state from before the
  outer transaction. Root commit makes the final state durable and publishes its effects. Unwinding
  must abort just as an ordinary failure does. If multiple writes at one position occur, the result
  and side effects must respect their documented order without duplicating drops or updates.
- While absent, an existing handle resolves to no entity. After abort it resolves to the
  restored entity and generation; after a committed replacement it stays stale. A new handle
  must be acquired for the replacement. A handle from an earlier world open remains invalid. A retained capability
  reference must not bypass these resolution and lifetime rules.

## Example: a chest

A loaded chest has items, a lock and an attachment, and a player is viewing it. A transaction
requests a valid block replacement that removes the chest. Within the transaction, block and
block-entity reads see the tentative replacement; the old chest's capabilities cannot be used. If a
nested transaction commits and the root then aborts, the original chest, items, lock, attachment,
block state and usable capability access are restored; the player has not seen a removal, received
items or had the menu closed by that aborted removal. The same restoration applies to an inner
abort, relative to the inner entry state.

If the root instead commits, the normal block replacement runs once, with its normal handling of
contents and viewers, and clients receive the resulting block and entity changes. The old chest is
no longer accessible through its old handle; if the new block has a block entity, acquire a new
handle for it. Actual drop and menu behavior must match the block's normal lifecycle rather than an
invented generic chest rule.

## Required implementation tests

1. Remove a populated, locked container with an attachment, commit, and compare block, contents,
   drop count, capability resolution, viewer closure and client notifications with an equivalent
   normal block replacement. Prove effects occur once.
2. Abort the same removal and assert the full original state, capability access and viewer state,
   with no drops, updates or notifications escaping. Repeat for an inner abort, a committed child
   followed by outer abort, and unwinding.
3. Exercise remove then replace, replace then remove, and multiple writes at one position, including
   queued entity updates; assert no stale update reaches a replacement and no effect is duplicated.
4. Check absent, unloaded, wrong-level and old-epoch handles, a replacement that creates a new
   entity, and refusal to retain a block requiring its entity. Verify position handles resolve as
   described both before and after abort and commit.
5. Cover block-entity absence and creation in the journal, attachment and capability state, and
   same-block lifecycle-preserving transitions. Exercise client and server observable outcomes,
   not only journal calls.

## Decisions for follow-up implementation

- Define the removal operation's replacement state and flags, including fluid preservation and
  whether the caller may choose a replacement; reject retaining a block that requires an entity.
- Extend the existing coordinator to journal existence and tentative visibility, while retaining
  the full `GameLevel::native_set_block` lifecycle at publication. Decide how ordered intermediate
  transitions and failure at publication are handled without leaking irreversible effects.
- Define how queued per-position entity updates are tied to the entity they describe, and how
  retained capability references revalidate after removal, replacement and abort.
- Make lifecycle effects, drops, menu closure and client notifications atomic with root commit;
  specify their order against other writes and prove it with the tests above.
- Coordinate container viewers and limits in [arismoko/horizon#975](https://github.com/arismoko/horizon/issues/975)
  and lock and replacement writes in [arismoko/horizon#977](https://github.com/arismoko/horizon/issues/977) with
  these existence, rollback and replacement rules; neither may introduce a second storage route.
