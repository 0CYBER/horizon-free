# Typed bulk block edits

Use complete `BlockState` values in `BlockEdit.palette`. Construct them with
`BlockState::of(&host.block_states(), block, constraints)` (or change an existing state
with `with`). `filters` contains owned `StatePredicate` values. `StatePredicate::block`
validates property names and values strictly; `StatePredicate::tag` retains arbitrary
property requirements, including for an empty tag, and rejects duplicate properties.

A predicate requires block identity or tag membership and every supplied property.
Omitted properties are wildcards, never default-state requirements. An unknown or
missing property on a tag member never matches. Sparse, non-Cartesian state tables
are supported: a partial predicate does not need to form a complete state with defaults.

All palette entries, predicates and run indices are prepared before any mutation.
NBT is not representable. At most 65,536 filters are allowed. Unloaded chunks and
out-of-height portions of runs are skipped. Edits preserve strict block-entity behavior,
notify no neighbors and send whole changed chunks.

## Read column palettes

`ColumnBlocks.palette` also contains complete `BlockState` values. States of the
same block with different properties have distinct entries. Only distinct numeric
states are converted to API values, not each block in the returned columns.

#### Upgrading saved block entities and entities

Data a mod reads from an older source (a schematic, a structure file, an old world's region) carries the
`DataVersion` it was saved at. `host.upgrade_block_entity_nbt(data_version, nbt)` and
`host.upgrade_entity_nbt(data_version, nbt)` run the game's own datafixes on one such value, the ones the chunk
loader runs on a block entity of a chunk of that version and the entity chunk loader on an entity (its passengers
included), up to the game's `DataVersion`. Both are server-side and pure: they read nothing from the level and change
nothing in it.

- Data at exactly the game's version comes back unchanged, and so does a block entity whose `id` its version
  doesn't know (as a chunk keeps one) and data the fixes can't read. Version 0 is a valid source.
- The fixes that need a whole chunk don't run: those that look at the blocks around a block entity, and the one that
  removes bed block entities.
- `Err` is an `NbtUpgradeError`, the same on native and wasm mods. Depth counts as the data is encoded: the root
  compound is at 0, and the compound the NBT writer puts around each element of a mixed list is a level of its
  own, so that element sits two levels below its list.
  - Refused before any fix runs, on the data you gave: `NegativeVersion`; `FutureVersion`, a version newer than the
    game's (the fixes only go forward); `TooDeep`, a list or compound 512 levels deep or more, the depth cap of
    every NBT read.
  - Refused after the fixes, on the upgraded data: `UpgradedTooDeep`. The data you gave was within the cap, but a
    fix made it deeper (1.9's `Riding` to `Passengers` doubles a chain's depth) past what any NBT read, the game's
    own included, could take back. Nothing is returned, so split or trim the data at its source.
- There is no size limit and no node cap, as saved data has none: nothing a world the game accepts could carry is
  refused for its size.
