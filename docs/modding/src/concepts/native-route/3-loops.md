# Published loops: `[[native.loop]]`

A published loop is a hot loop of the game with a **slot**: before and after it handles each item, the loop
asks your mod's type what to do. You fill the slot with a `[[native.loop]]` row; the game publishes the loop
`block_entities.tick`.

Use a slot to change how individual block entities tick, at tick speed. To run code once around the whole
block entity tick, an [`[[inject]]` listener](../injections.md) or a `HEAD` mixin on the loop is enough.

## Example

This slot reduces a hopper's cooldown by one and skips its tick when the cooldown exceeds one.
Add the row to your native mod's manifest, keeping its header and `[native]` pins.
Slots with a priority below 20 run before this slot.

```toml
# Reduce the cooldown and skip the tick while a hopper cools down.
[[native.loop]]
id = "throttle"
loop = "block_entities.tick"
helper = "src/loops/throttle.rs"
with = "Throttle"
priority = 20
```

`src/loops/throttle.rs`:

```rust,ignore
/// While a hopper cools down, takes one tick off its real cooldown and skips its tick.
#[derive(Default)]
pub struct Throttle {
    skipped: u64,
}

impl BlockEntityTickSlot for Throttle {
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        if tick.kind != TickedKind::Hopper {
            return LoopFlow::Continue;
        }
        match tick.game.hoppers.get_mut(&tick.pos) {
            Some(state) if state.cooldown > 1 => {
                state.cooldown -= 1;
                self.skipped += 1;
                LoopFlow::Skip
            }
            _ => LoopFlow::Continue,
        }
    }
}
```

Unlike a mixin helper, this file may hold any Rust items, `pub` or not, with no `use` lines. It sees what the module
`horizon_client::game::block_entity_ticks` sees: `BlockEntityTickSlot`, `BlockEntityTick`, `LoopFlow`,
`TickedKind`, `OwnBlockEntities`, `Game`, `BlockEntity` and the rest. Like a [mixin helper](2-mixins.md#writing-a-helper), it may
call your mod's crate by its Rust crate name.

## Fields

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | The row's name, unique in the mod. |
| `loop` | yes | A loop the game publishes: `block_entities.tick`. |
| `helper` | yes | A `.rs` file under `src/` with the slot's code. |
| `with` | yes | The slot type the helper defines. It implements the loop's slot trait and `Default`, and is `'static`. |
| `priority` | no | **Lower** is asked first (the opposite of mixins); ties go by mod id, then row id. The default is 1000. |

## What you will see

The build log shows `Published loops: 1 filled by 1 mod slots`, with your numbers, beside the
`Function replacement:` line.

## The `block_entities.tick` slot

This is the server's block entity tick: hoppers, crafters, jukeboxes, beacons, creaking hearts, spawners, trial
spawners, vaults, beehives, furnaces, brewing stands, daylight detectors, end gateways, campfires, potent
sulfur and mods' ticking block entities. It runs only where a world is simulated: a dedicated server, a LAN
host, and in single player the game's built-in server. A client connected to a server never runs it.

The slot trait `BlockEntityTickSlot` has two methods, and both do nothing by default:

- `fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow` runs just before each block entity
  ticks;
- `fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>)` runs just after it ticked.

`before` sees only the block entities vanilla would tick: in a block-ticking chunk, inside the world border,
live and not asleep. The cursor `tick` has:

| Member | What it is |
| --- | --- |
| `game` | `&mut Game`: the whole game, including its level and its `hoppers`. |
| `pos` | The block entity's position. |
| `kind` | A `TickedKind`: `Hopper`, `Crafter`, `Jukebox`, `Beacon`, `CreakingHeart`, `Spawner`, `TrialSpawner`, `Vault`, `Beehive`, `Furnace`, `BrewingStand`, `DaylightDetector`, `EndGateway`, `Campfire`, `PotentSulfur`, or `Mod` for any mod's block entity. |
| `block_entity()` | The block entity itself, `Option<&mut BlockEntity>`; `None` if it is gone from the level. |

| `before` returns | Effect |
| --- | --- |
| `LoopFlow::Continue` | The entity ticks, once every other mod's slot also continues. |
| `LoopFlow::Skip` | The entity does not tick this tick, and no `after` runs for it. |
| `LoopFlow::Stop` | No further entity ticks this tick. The work after the loop runs, such as shulker box lids. |

**Your own mod's block entities.** `TickedKind::Mod` covers every mod's. To pick out yours, keep an
`OwnBlockEntities` in your slot, made from the ids you registered your block entity types under, and ask it
about each entry. For example, this slot counts block entities registered as `mymod:counter`:

```rust,ignore
/// Counts this mod's counters among the block entities the loop visits, and always answers Continue.
pub struct Own {
    own: OwnBlockEntities,
    counted: u64,
}

impl Default for Own {
    fn default() -> Self {
        Self {
            own: OwnBlockEntities::new(&["mymod:counter"]),
            counted: 0,
        }
    }
}

impl BlockEntityTickSlot for Own {
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        if self.own.contains(tick) {
            self.counted += 1;
        }
        LoopFlow::Continue
    }
}
```

| `OwnBlockEntities` | What it does |
| --- | --- |
| `const fn new(ids: &'static [&'static str]) -> Self` | Your types, such as `&["mymod:machine"]`. Resolution happens on the first visit to a mod block entity. |
| `fn contains(&mut self, tick: &BlockEntityTick<'_, '_>) -> bool` | Whether the visited block entity is a mod's of one of your types. |

It answers for the block entity itself, whatever block it stands on: one of your types on another mod's
block is yours, and another mod's on your block is not. Every mod block entity carries its type's numeric
wire ID, looked up once when it is made or loaded. The first `contains` that visits a mod block entity resolves your ids once into a flag
for each of those IDs; every call after it reads one flag, with no string compare, no hashing and no
allocation, so it costs the same on every tick. Vanilla block entities answer `false`.
The type table is fixed for the process. A block entity with no numeric type ID matches nothing.

An id the game has no block entity type of (misspelt, or a mod that is not loaded) matches nothing. The first resolution logs a warning naming it: ``Published loop block_entities.tick: OwnBlockEntities names
`mymod:machin`, which is no registered block entity type; it matches nothing``.

To read or change your block entity, match `tick.block_entity()` against `Some(BlockEntity::Mod(entity))`.
Its saved data is `entity.nbt`, its item slots `entity.items` and its menu data `entity.data`.

## What you can rely on

- Slots are asked in priority order. The first answer that is not `Continue` wins, and later slots are not
  asked about that entity. For an entity that ticked, `after` runs for every slot.
- Within one entry, every slot's `before` gets the same cursor: if an earlier slot replaced the entity and
  continued, a later slot's `before` sees the `kind` and type from before that edit (the loop then
  rechecks and skips the replaced entity, so it doesn't tick as the old one).
- Your slot value is made with `Default` on the loop's first run and kept. There is one per `Game`, the
  game state of one running server, which ticks every dimension in turn: read `tick.game.dim` to tell them apart.
- A slot may add, remove or replace block entities. If it removes the visited one, or replaces it with another
  `kind` or a mod block entity of another type, that one does not tick. One removed later in the list is passed over. One added ticks from the next pass.
- Calling `tick.game.tick_block_entities()` from inside a slot runs the game's own loop, without any slots,
  as a whole pass.
- A panic in a slot is reported naming your mod and the loop: `in native mod <id>, hook loop
  block_entities.tick, part <id>::<Type>`. The panic unwinds the call. Subsequent calls on that `Game` run the loop without any mod's slots.

## Mixins on the loop

The loop's entry is `horizon_client::game::block_entity_ticks::Game<'_>::tick_block_entities`. A `HEAD` or
`RETURN` mixin and an `[[inject]]` listener on it run around the loop. To change a place inside the loop,
target the loop's body, `…::tick_block_entities_with`. Such a mixin does not apply when the loop runs
without slots, from inside a slot or after a panic.

## Errors

| Code | Message | Fix |
| --- | --- | --- |
| `E_LOOP_WASM` | `<mod>:<row>: <loop>: [[native.loop]] compiles game code; only native mods can fill a loop` | Build the mod native. |
| `E_LOOP_INVALID` | `<mod>:<row>: invalid loop; name a published loop of the game, such as block_entities.tick` | Name the loop. |
| | `… invalid helper; name a Rust file under src/ in this package` | Put the helper under `src/`, ending in `.rs`. |
| | `… invalid with; name the slot type the helper defines` | Write the type's name, a plain identifier. |
| `E_LOOP_UNKNOWN` | ``<mod>:<row>: the game publishes no loop `<loop>`; its loops: block_entities.tick`` | Use a listed loop. |
| `E_LOOP_HELPER` | `<entry>: <mod>:<row>: its helper: <parse error>`, or `with: <parse error>` | Fix the Rust error shown. |
| `E_LOOP_ENTRY_REPLACED` | `<mod>:<row>: the <KIND> mixin on <target> would replace or bypass the published loop …; only HEAD and RETURN mixins compose with it` | On the entry, use `HEAD` or `RETURN`. Never `OVERWRITE` the body. |

A helper that does not compile is reported with the compiler's error under your mod's id.
