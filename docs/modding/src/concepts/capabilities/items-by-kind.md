# Items by kind

Use `STORAGE` (`horizon:storage`) to move items by kind in bulk, rather than by slot.
Use `ITEMS` (`Capabilities.Item.BLOCK` in NeoForge) when you need slot-indexed access.
NeoForge 26.3's transfer API has no by-kind counterpart to `STORAGE`.

A block's `ItemStorage` answers by kind, with `u64` amounts:

- `amount(host, resource)` and `space_for(host, resource)`: how much of one kind it holds, and how much an insert
  would take (a figure to plan by, not a promise).
- `insert(host, tx, resource, amount)` and `extract(...)`: writes in the caller's transaction, never more than asked.
- `page(host, cursor, page)`: one page of what it holds. The cursor is a plain value the caller keeps (0 for the
  start); the answer is the cursor to ask for next, or `None` at the end. A listing that changes between pages can skip a kind or return it twice.

**Who answers it.** The game answers `STORAGE` for every container or composter port `ITEMS` reaches. Its own containers are asked in
one pass over the slots the side reaches, by the rules hoppers follow, so a chest, a hopper, a composter and a furnace give the same
answers through either capability. A container whose `ITEMS` a mod answers is wrapped slot by slot. A mod's bulk store
(a drive, drawers, a vault) answers `STORAGE` with a provider of its own, one entry per kind however many items it
holds: register it `before_game` if the block also has slots.

## Moving items

 `transfer(host, from, to, resource, max, tx)` moves one kind as one step of `move_resources`: all or
nothing, a partial take retried, and `StorageError::Overanswered` when a handler answers more than it was asked. It queries the source amount and destination space before extracting and inserting.
A partial move can require additional calls. `transfer_matching(host, from, to, filter, max, tx)` moves up to `max` items accepted by
the filter, listing the source a page at a time.

**Limits the host keeps.** A `StoragePage` holds at most `MAX_STORAGE_PAGE` (256) entries whatever limit a caller asks,
and a limit of 0 is refused with `StorageError::ZeroPage`. A handler cannot push past its page, and a page crossing the
wasm boundary is checked again when it is read. Totals and cursors that do not fit in 64 bits are refused with
`StorageError::Overflow`, never wrapped.

**Cost.** A bulk store can use a by-kind index to avoid scanning slots. The game's adapter answers in one pass over the slots
(O(slots)), with no work per item moved, so a question about a chest costs the same whether it moves one item or a
stack. A wrapped mod `ITEMS` handler costs one handler call per slot.

## Choosing a capability

Use `ITEMS` and `move_resources` for slot-specific operations and small transfers between
ordinary containers. Use `STORAGE` for bulk stores and by-kind routing. The cost depends on
the provider and the slots it exposes; `STORAGE` does not guarantee constant-time access.
