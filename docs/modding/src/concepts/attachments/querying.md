# Query attachment holders

Use `AttachmentQueries` to find loaded holders of an `AttachmentType<T>` without
checking every entity, chunk or block entity. Declare your key as described in
[server attachments](../attachments.md). Import `AttachmentQueriesMut` for server
writes. Both traits expose the same API on native and wasm builds.

## Read a snapshot

Call `host.query(&TYPE)` on `&dyn Host`, then borrow rows with `batch.iter()`.
Each successful row contains a copyable `holder` and a borrowed `value: &T`.
Match `AttachmentHolder` to obtain an `EntityRef`, `PlayerRef`, `BlockEntityRef`,
`ChunkRef` or `LevelRef`.

For example, sum a counter across its holders:

```rust,ignore
use mod_api::*;

static VALUE: AttachmentType<u32> = AttachmentType {
    namespace: "example",
    name: "value",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

fn total(host: &dyn Host) -> Result<u32, AttachmentRowError> {
    let batch = host.query(&VALUE).map_err(AttachmentRowError::Value)?;
    let mut total = 0;
    for row in batch.iter() {
        total += *row?.value;
    }
    Ok(total)
}
```

| Rule | Behaviour |
|---|---|
| Value bounds | Iteration requires `T: Send + Sync + 'static`, not `Clone`. |
| Snapshot | Membership and stored value bindings are captured at query creation. Create a fresh query to see later sets, removals, loads or unloads. |
| Borrowing | Rows borrow from the batch, not the host, and cannot outlive the batch. You can use the host while retaining a batch. |
| Defaults | Queries never initialize defaults, dirty holders or queue synchronization. An absent key produces an empty batch. |
| Saved data | Saved values on bound holders decode lazily when visited, without an earlier typed read. Malformed data and incompatible Rust types produce row errors instead of defaults. |
| Interior mutability | A snapshot does not deep-freeze interior mutability inside `T`. |

For an immutable keyed read, including in a render hook, call
`query.get(holder)?` with an `AttachmentHolder`. This requires `T: Clone` and
returns an owned `Option<T>`. The expected O(1) lookup precedes decoding and does
not scan rows. Decode and type errors use `AttachmentRowError::Value`.

`Ok(None)` means the holder is absent from the snapshot, including when its data
has not arrived through synchronization. It does not prove the holder is alive:
`get` performs no host liveness check. A retained query reads its captured value
even after a live replacement or removal.

## Write explicitly on the server

Call `host.query_mut(&TYPE)` on `&mut dyn ServerHost`. Values require
`T: Clone + Send + Sync + 'static`. Each row provides `holder()`, `value()`,
`set(host, replacement)` and `remove(host)`; it does not provide `&mut T`.

```rust,ignore
fn increment(host: &mut dyn ServerHost) -> Result<(), AttachmentRowError> {
    let batch = host.query_mut(&VALUE).map_err(AttachmentRowError::Value)?;
    for row in batch.iter() {
        let row = row?;
        row.set(host, *row.value() + 1)?;
    }
    Ok(())
}
```

- Writes take effect through the ordinary attachment write path. Captured rows
  retain their original values; use a fresh query to observe a write.
- Each write validates the captured residency against the supplied host's current
  dimension. Unloaded, replaced or missing holders produce
  `AttachmentRowError::StaleHolder`. A host for another dimension does not
  retarget the row, and a refused write does not revive the holder.
- Removing only the attachment value does not invalidate a loaded holder. A
  retained row can set a replacement value on that holder.
- Writes join your existing transaction. For an all-or-nothing loop, import
  `HostTransaction` and use `host.transaction`, as described in the
  [transaction guide](../transactions.md).
  There is no implicit transaction or commit on drop. Dropping a batch neither
  applies deferred changes nor undoes successful writes.
- `remove` returns the ordinary attachment operation's `Option<T>` and preserves
  its codec errors.

## Scope and order

Queries cover only the host's current dimension and do not load chunks or
entities. Copying a holder identity does not keep it loaded. Attachments belong
to an entity object, not its saved UUID: equal-UUID objects have separate values.
An entity that is not admitted to the level is absent from queries.

A query is a residency view, not an immediate liveness filter. An entity queued
for removal can appear until cleanup runs. Its handle's stale rules apply, and
stale writes fail. Do not convert a scoped handle to an unscoped storage ID to
bypass those checks.

Traversal uses dense index order: insertion appends, and removal fills its slot
with the final row. The same ordered mutation history produces deterministic
order, but that order is neither sorted nor stable across structural changes.

## Costs and type identity

Native acquisition performs one indexed lookup and clones a bucket `Arc`, not
`T`. Empty native acquisition allocates nothing. Warm borrowed iteration makes
no allocations; a first saved decode can allocate in the codec and read cache.

Retaining a snapshot pins its old values. A subsequent live mutation can copy
row metadata and shared pointers, without cloning each typed value. Retaining
many batches or interleaving snapshots and writes increases storage and copying
costs. No index sweep runs without an attachment or residency change.

A decoded value requires matching Rust type identity. An incompatible type
produces `AttachmentError::TypeMismatch`, without falling back to saved decoding.
Share types through your mod dependencies rather than declaring a different
type with the same attachment key.

## Store errors

A first store on a resident mob, painting or dragon part registers its attachment
lifetime. Each dragon part has independent contents. Removing, replacing or
unloading the dragon retires its parts even when snapshots retain their values;
parking a level preserves its resident parts. Stores in construction listeners
are indexed even when inspection triggered construction. Inspection and removal
do not themselves register a holder.

| Error | Meaning and action |
|---|---|
| `AttachmentError::HolderBusy` | Physical storage is temporarily unavailable during an engine pass. Retry after the pass. The refused store writes no value and registers no holder. |
| `AttachmentError::UnsupportedHolder(family)` | The physical family cannot own attachments. Retrying does not help. `family` is a lowercase identifier, not an engine type. |
| `AttachmentRowError::HolderBusy` / `AttachmentRowError::UnsupportedHolder(family)` | The corresponding error from a retained row write. |
| `AttachmentRowError::StaleHolder` | The row's holder is missing or its captured residency is invalid. |

The `UnsupportedHolder` family text is limited to 256 UTF-8 bytes at the wire
boundary; longer names are rejected on native and wasm guest paths.
