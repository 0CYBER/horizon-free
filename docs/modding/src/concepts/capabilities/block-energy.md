# Block energy

Use `level.capability(host, &ENERGY, pos, side)` to look up a block's
`EnergyHandler`. Transfer energy inside a transaction with
`move_energy(host, &from, &to, max, tx)`. It returns the amount moved or
[`Overanswered`](answer-limits.md). A partially accepted move is rolled back and
retried with the accepted amount.

`ENERGY` corresponds to NeoForge's `Capabilities.Energy.BLOCK`. An `EnergyHandler`
reports an amount and a capacity, without a resource identity. Its `insert` and
`extract` methods receive a `Transaction` and return the amount transferred.

## Block providers

Register a provider with `r.block_entity_capability(&ENERGY, &battery, …)`, using a
declared or looked-up block entity type key. Alternatively, declare the game's
buffer with `r.block_energy(&block, buffer)`, where `buffer` is a `ModEnergyBuffer`:

- Insertion respects the input sides, remaining capacity and per-insert limit.
- Extraction respects the output sides, stored amount and per-extract limit.
- A side that permits neither operation returns no handler.

## Reusable handlers

| Handler | Behaviour |
| --- | --- |
| `EmptyEnergyHandler` | Holds 0 of 0 and refuses insertion and extraction. |
| `InfiniteEnergyHandler` | Supplies every requested extraction and accepts no insertion. |
| `VoidingEnergyHandler` | Accepts every insertion and stores nothing. |
| `LimitingEnergyHandler::new(inner, max_insert, max_extract)` | Clamps each request before forwarding it in the same transaction. |

To delegate, implement `EnergyHandler` on a struct that holds another handler and
forward its methods. Journal any additional state you change in a transfer so
an aborted transaction restores it.

Energy handlers, including those reached through `Capability`, provide `is_full`
and `comparator_signal`. The signal is 0 for an empty or zero-capacity handler;
otherwise it is `1 + floor(14 * min(amount, capacity) / capacity)`.

## Item energy and component storage

Look up `ITEM_ENERGY` with `place.capability(host, &ITEM_ENERGY)` and register a
provider with `r.item_capability(&ITEM_ENERGY, "mymod:cell", …)`. Vanilla items
have no energy provider. Use `move_energy` for transfers between item and block
handlers.

`ItemComponentEnergy::new(place, component, capacity, max_insert, max_extract)`
stores energy in an integer item component; an absent component reads as 0.
Changes exchange one item at its `ItemPlace`, splitting the changed item from a
larger stack.

For fluids, `ItemComponentFluidTank::new(place, component, capacity)` stores a
single tank in a compound component, such as
`{id: "minecraft:water", amount: 1000, components: {…}}`. It removes the component
when the tank empties. Register it under `ITEM_FLUIDS`.

Both handlers use `ItemComponentHandler` to read a component at a place and
rewrite it through an exchange. You can reuse that component access for your own
resource storage.
