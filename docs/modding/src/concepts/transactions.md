# Transactions

Use `HostTransaction` on a `ServerHost` to group capability writes. The closure's
`Ok` commits; `Err` aborts. Pass the borrowed `&Transaction` to each handler or
place write.

```rust,ignore
use mod_api::{HostTransaction, ServerHost};

fn update(host: &mut dyn ServerHost) -> Result<(), ()> {
    host.transaction(|host, tx| {
        // Pass host and tx to your capability writes.
        Ok(())
    })
}
```

## Commit and rollback

- Writes apply immediately, so subsequent steps in the transaction see them.
- The game snapshots each storage before its first write. Handlers use the game's
  journal rather than keeping their own.
- A nested commit merges into its parent. Only the outermost commit makes changes
  final and publishes their notifications.
- An abort restores the recorded storage without publishing commit effects.
- A panic unwinding out of the closure aborts its transaction. The parent remains
  open if you catch the panic inside the parent closure.
- Closing a transaction aborts any transactions left open inside it before closing
  the parent.

The journal covers writes through capability handlers to these storages:

| Storage | Recorded state |
|---|---|
| Block entity | Item slots, saved compound, custom name and attachments |
| Hopper | Cooldown |
| Cauldron | Block state |
| Composter | Block state and composter-local random stream; fill feedback, game events and maturation scheduling wait for outer commit |
| World fluid pickup and placement | Block state, and a block entity the new block removes |
| Player | Inventory slots, armour, offhand, body, saddle, cursor, ender chest and persistent data, for host and guest players |
| Entity inventory | Chest or hopper minecart, chest boat, chested donkey, mule or llama inventory |
| Item entity | Stack; an emptied entity remains until the outermost commit removes it |

At the outermost commit, the game publishes change notifications and deferred
effects: `setChanged`, comparators, jukebox playback, cauldron and composter block updates and
item drops. Drops include items placed at a block, thrown by a player or entity,
or split off an item entity.

A world pickup or placement of fluid (`level.pickup_fluid`, `level.place_fluid`) writes
its block at the call and the rest at the outermost commit; an aborted one leaves the world
unchanged, where NeoForge's may not. `level.play_fluid_effects` queues a fluid's sound and
game event the same way: they play only at the outermost commit and are dropped on abort,
unlike `trigger_sound_and_game_event`, which acts at once. The `block_drops` listeners of a
replaced block see it as it was, and a cancelled drop keeps the replacement.

Direct writes are not journaled: direct item-entity stack writes,
`LivingRef::set_equipment`, and the armor-stand `slot_set` paths take
effect immediately, even inside a transaction. Equipment sounds and equip/unequip
events from `LivingRef::set_equipment` also happen immediately. An abort does not
undo these writes or effects. Use journaled capability handlers when you need rollback.

## Spawning and removing entities

Entities a transaction spawns or removes follow the same commit and rollback rules:

- A spawn builds the entity at the call: its handle works, its attachments and Parts exist,
  `entity_constructing` has run and selectors find it. Its `entity_join_level` does not run
  until the outermost commit, in the order the entities were added, before any removal
  of the transaction takes effect. A listener reads the entity at the commit, not during the
  call. A join a listener cancels at the commit means the entity was never announced and the
  handle the spawn returned reads stale from then on.
- An abort takes out every entity the transaction spawned, with its attachments and saved data, and
  tells no mod: neither `entity_join_level` nor `entity_leave_level` runs. An entity id is
  never reused, so the aborted entity's id stays burnt and a stale handle never names a later entity.
- A removal (`EntityRef::discard`, a kill) marks the entity at the call: its handle and selectors
  stop finding it, and a mob reads as removed (it is flagged at the call; any other entity is
  listed as removed until the commit has run its removal, joins told by then included). Passengers dismount, the leave
  is told and the other effects of removal wait for the outermost commit; an abort puts the
  entity back as it was and tells no one. An entity spawned and removed in the same
  transaction is never announced, so it never leaves.
- A spawn is refused, inside a transaction as outside one, once the entity ids run out.
- A transaction covers the entities of the level it was opened in; with transactions of several levels open (a command run `in` another level can open one), an entity belongs to the innermost one of its own level. A command it runs in another
  level (`execute in`) spawns or removes there as it would outside a transaction: the join and
  the leave are told at once, and an abort does not take the entity back. A join listener that
  spawns, removes or commits a transaction while a commit is telling its joins is told about its
  own entities after the commit's batch, in the order it asked.
- Travel between levels is not journaled: an entity that arrives in another level during an aborted
  transaction stays where it arrived.

## Transaction authority

`Transaction` identifies an open transaction with a game-issued write token. It
does not grant permission to close it. A write with a token that does not name an
open transaction changes nothing, including a token retained past its closure.

Closing requires a separate `TransactionSeal`. The `host.transaction` guard holds
that seal; its closure receives only the write token. If you use the raw
`transaction_open` method, you receive the seal and must close it with
`transaction_close`. The game refuses a seal that does not match an open
transaction and logs `Horizon: refused to close a transaction the seal does not end`.

## Inspecting a transaction

These methods are server-only:

| Method | Result |
|---|---|
| `host.transaction_current()` | Innermost open `Transaction`, or `None`; not a closing seal |
| `host.transaction_depth(tx)` | `Some(0)` for the outermost transaction, increasing by one per nesting level; `None` if it is not open |
| `host.transaction_lifecycle()` | `TransactionLifecycle` describing the open or closing phase |

| `TransactionLifecycle` variant | Meaning |
|---|---|
| `None` | No transaction is open |
| `Open` | A transaction is open and none is closing |
| `Closing` | A nested transaction is committing into its parent or aborting |
| `RootClosing` | The outermost transaction is closing |

On a root commit, `contents_changed` listeners run during `RootClosing`. Their
writes are outside the transaction whose commit triggered them.

## Entity inventory slots

`ItemPlace::EntitySlot { entity, slot }` addresses an entity inventory slot, like
NeoForge's `Capabilities.Item.ENTITY`. Read and write it like any other item place.
Reading or writing a minecart's slots unpacks its loot table without a player,
as vanilla's `getChestVehicleItem` does.

The player `set_inventory_slot`, `set_ender_chest_slot`, `give_stack` and
`set_persistent_data` writes use the open transaction journal. Aborting restores
the old values; equipment effects and given-stack overflow wait for root commit.
