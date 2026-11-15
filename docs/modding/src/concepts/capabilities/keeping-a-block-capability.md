# Keeping a block capability

A pipe that asks a neighbour for `ITEMS` every tick doesn't have to look the capability up every
tick. `BlockCapabilityCache` keeps what one lookup found and gives it back while nothing the answer
depends on has changed. It is the caching half of NeoForge's `BlockCapabilityCache`, without the
listener: the cache is polled, never told.

## Making one

```rust,ignore
let mut west = BlockCapabilityCache::new(&ITEMS, level, [x - 1, y, z], Some(Direction::East));
```

`new` looks nothing up. Keep the cache in your block entity's part or your machine's state, one per
neighbour and side you ask. Each cache holds at most one handler, the last one found, so what it
keeps is bounded by the number of caches you make.

## Polling it

```rust,ignore
// Each tick, inside a transaction `tx`, with `own` your own handler.
if let Some(items) = west.get(host) {
    move_resources(host, items, own, |_| true, 8, tx)?;
}
```

`get` gives the kept handler while all of these hold, compared in constant time with what
they were at the lookup:

- **The level is as in place as it was** (`LevelRef::in_place`). A parked level lends nothing, as a
  fresh lookup there finds nothing, and once its dimension is ticking again the cache asks again.
- **The block entity at the position is the same one.** Its handle carries a generation, so a block
  entity that was removed, replaced or unloaded with its chunk compares unequal.
- **The block state there is the same.** A chest that was turned is asked again.
- **No provider said its answer there changed.** The level keeps a capability generation per
  position (`level.capability_generation(host, pos)`), which `invalidate_capabilities` moves on;
  see below.
- **No mod was switched on or off since.** The count comes from the game through the host, so a
  wasm mod's cache sees the same switches as a native one.

When any of these differs, `get` looks the capability up again with `LevelRef::capability`. A
successful poll allocates nothing and asks no provider.

## When a level isn't there

A level that closed leaves its handles stale: `get` finds nothing for it, and a new world gives you
new `LevelRef`s to build new caches with. A parked level finds nothing until it is in place again.

## When a provider's answer changes

A provider whose answer changes with none of the above, such as a machine whose sides can be
configured, tells every cache at its position at once:

```rust,ignore
// The machine's side configuration changed.
level.invalidate_capabilities(host, pos);
```

This is NeoForge's `ILevelExtension.invalidateCapabilities(pos)`. It moves the capability
generation at `pos` on, and each cache there looks up again at its next `get`, whichever mod keeps
it. Inside a transaction the generation moves at the root commit; an aborted transaction leaves it
as it was, so a cache never looks again for a change that didn't happen. A mod can only move a
generation on: it can't set or read-and-change the number, and only a change of it means
anything. It returns `false` and changes nothing off the level in place, for a stale handle, on a client and for a position
that isn't loaded.

The level keeps a generation only for a position a provider invalidated, and drops it with its
chunk; a level that closes takes them all. A position never reads a number it read before while
its level is open. When a chunk that held generations unloads, the level's other positions read a
new number too, so their caches each look up once more after it: a lookup too many, never a stale
answer.

## Forgetting a kept handler

`invalidate()` on a cache forgets what it keeps, so its next `get` looks up again. It is for the
cache's own owner, such as a pipe that was reconfigured; a provider uses
`invalidate_capabilities`, which reaches every cache at its position.
