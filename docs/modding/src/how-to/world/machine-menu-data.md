# Declaring a machine's menu data

A mod block entity has two different kinds of storage: external item access uses
transactional Parts, while internal slots use the handle methods below; integer menu data is read with
`BlockEntityRef::data_slots` and changed on a server Host. Menu data is transient:
creation and loading both start with an empty data vector, even when the saved
payload contains the machine's progress.

## State-aware creation

Declare the block entity type and its owning blocks during registration. To make
creation depend on block properties, answer `stateful_block_entity` with `true` for
that block. The `block_entity` hook then receives the actual placed
state, not just the block identity. Read its properties through the Host's state
registry and supply the initial saved compound. The registered declaration owns
the entity type; an unanswered state-aware creation makes no entity.

Creation runs inside the block write, after placement side effects, on servers
and clients. Changing properties within the same block preserves an existing
entity even when a fresh instance of the new state would have no entity.

## Size the data vector

Call `entity.set_data_slots(host, initial_values)` with the complete initial
vector when preparing a newly created or loaded machine. For a progress display,
use one element initialized from the progress value in its saved payload.
Do not reset the vector to its initial values on every tick.

The method accepts at most the game's mod-menu data-slot limit (256).
It returns `Ok(true)` when it changes the vector, `Ok(false)` for identical values,
or `BlockEntityDataError::TooManySlots` with the submitted count and limit. It
never silently clamps a vector. `BlockEntityDataError::Unavailable` means the
handle cannot reach a writable mod holder in the server level in place.

## Update individual values

After sizing, call `entity.set_data_slot(host, index, value)` for scalar updates.
This method does not resize: an absent index returns `Ok(false)`. Both methods use the
same write target, journal and `setChanged` path. Nested transactions retain their
ordinary behavior; an aborted outer transaction restores the vector that existed
before its first write, including writes committed by children. The normal menu
broadcast delivers committed changes to viewers.

The public handle resolves one entity's lifecycle generation as well as its level
and position. Removal, replacement and chunk reload make it stale; in-place data
edits preserve it. It is not a borrowed-payload API.

## Internal items versus automation

For a configured mod container, `item_slot(host, slot)` snapshots an internal slot;
`set_item_slot(host, tx, slot, stack)` replaces it within an existing server
transaction. Any held mod block-entity handle can be used; there is no caller
ownership check. This deliberately bypasses inputs, outputs and face policy, so a
recipe can consume its input and produce its output atomically, including output
slots that automation cannot insert into. Use the `ITEMS` capability for external
insertion; it enforces automation policy. Vanilla containers are unavailable here.

Indices use the current container's declared count. Lazy slots read empty; saved
excess slots are neither exposed nor truncated. `None` clears, identical values
return `Ok(false)`, and real changes send `Changed` at root commit. Nested abort
restores ordinary before-images, including committed-child writes. Invalid
indices, unknown/air/zero-count stacks and counts above the registered or
component-overridden maximum return `BlockEntityItemError`, never a partial write
or silent count clamp. A stale handle refuses a replacement rather than accessing
its container; acquire a new handle for the replacement.

## Skip unchanged tick write-back

The existing `BlockEntityPart::block_entity_tick` event has the same reply on native
and wasm. It defaults to `Changed`, retaining full checked payload write-back.
When no payload should be written back, set:

```rust,ignore
*event.reply = BlockEntityTickReply::Unchanged;
```

This discards all edits to `event.entity`, omits the reply snapshot encode/decode,
preserves the original revision, and causes no tick-only chunk dirty mark or
container/comparator effects. Removal or replacement during the callback prevents
restoring the original entity. Independent Host writes still have their own normal
effects. The input snapshot is still sent; this is not batching or a validity cache.

## One-shot client effects

To tell clients about something that happened once, such as a burst of particles, queue
fields instead of writing them to the saved `nbt`. In `block_entity_tick`, or another
event with a mutable entity, native and wasm mods alike call
`event.entity.set_client_update_fields(fields)`, then `host.block_entity_sync(event.pos)`.
A later call replaces the batch; an empty compound clears it.

The next block change that reaches a viewer puts the fields over what
`block_entity_update_tag` gives, or sends them alone without one, in one packet for every
current viewer, and consumes the batch. A change nobody sees leaves it waiting, and neither a
chunk sent to a player nor a resend of the blocks under a single player takes it. Removing
or unloading the block entity drops it. It is never saved, cloned or compared, and it does
not change the entity's revision.

If the merged tag exceeds the 2 MiB network-NBT limit, the game drops the fields,
logs a warning, and sends the original update tag instead. A transaction that rolls
the block entity back also drops any batch queued before it.

A client merges the fields into its copy's `nbt` as any update field, and they stay there
until sent again: put a counter in them to tell one effect from the next.

## Retaining a handle

A `BlockEntityRef` names one live entity, not every entity later placed at its position. Removal,
replacement and chunk reload invalidate it; reacquire with `host.block_entity(level, position)`.
In-place data edits and temporary tick storage preserve identity. Option reads return `None` for
stale handles; writes return their error's `Stale` variant, never writing to a replacement.
`set_lock`, `set_name`, `merge_data`, `replace_data`, `set_changed` and `set_data_slot`
return `Result<bool, BlockEntityError>`: handle `Err(BlockEntityError::Stale)` by dropping
or reacquiring the handle. `Ok(false)` means unchanged data or an unsupported operation,
not a stale identity. Item and data-vector methods return `BlockEntityItemError` and
`BlockEntityDataError`, respectively.

## When its chunk unloads

A machine that keeps something outside its saved data (a running job, an open handle, its place in an
energy or pipe network) lets go of it in `on_chunk_unloaded`, NeoForge's `onChunkUnloaded`: a
`BlockEntityPart` on the type is told once for each of its block entities in a chunk that unloads, on each
side that holds a copy, before the copy is dropped.

```rust,ignore
struct Node;
#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Node {
    fn on_chunk_unloaded(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::OnChunkUnloadedEvent<'_>,
    ) {
        if let mod_api::HostSide::Server(_) = host {
            // The handle equals the one `host.block_entity` gave while it was loaded.
            NETWORK.lock().unwrap().remove(&event.block_entity);
        }
    }
}
// In Hooks::register:
// registrar.block_entity_part(&node_type, Node)?;
```

`event.pos` is where it stands and `event.entity` its data at the call: on a server what the chunk was
just saved with, on a client its copy as the server last sent it. On a server the chunk is saved before
the call, so a change made in it is not kept, and the chunk may already have left the level, so its
handle's reads find nothing there. It is not called when the world closes, nor on a client when its whole
level goes, and never for a vanilla block entity. The chunk-wide notice is `chunk_unload`, a
server hook told after the chunk's block entities; a client is told of the block entities alone
([hook notes](https://github.com/arismoko/horizon/blob/main/crates/horizon-mod-vocab/src/hooks/on_chunk_unloaded.rs)).

## Client ticks: a machine's hum

What a running machine does on the client alone (a looping hum, smoke from its chimney, a part eased between
two server updates) hangs off its client tick, vanilla's client `BlockEntityTicker`. A `BlockEntityPart` opts
its type in with `block_entity_client_ticks`, asked once for each of its blocks, and is then told
`block_entity_client_tick` once per client tick for each copy this client holds, while the level runs (not
paused or frozen), in vanilla's ticker order. A loaded copy ticks whether or not it is drawn: hidden behind a
wall, out of view or beyond `block_entity_view_distance`, where `block_entity_submit` is not called. It is
never called on a server, and a type that doesn't opt in costs nothing.

The event has the copy's `pos`, its `block_entity` handle, its block `state` and its `entity` data, read only,
and the host is a `ClientHost`. The data is what the frame's handling of the server's packets merged into the
copy (`block_entity_update_tag`, and `block_entity_data_received` told before): a frame handles what has
arrived, then runs its client ticks, so every tick of one frame sees the same data, and an update that arrives
during a frame is seen from the next frame's first tick.

Here a machine whose update tag carries `running` starts a looping hum on the first client tick it runs and
fades it out when it stops:

```rust,ignore
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The hums playing, by the machine they belong to.
static HUMS: LazyLock<Mutex<HashMap<mod_api::BlockEntityRef, mod_api::SoundRef>>> =
    LazyLock::new(Default::default);

struct Hum;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Hum {}
#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Hum {
    fn block_entity_client_ticks(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::BlockEntityClientTicksEvent<'_>,
    ) {
        event.ticks = true;
    }
    fn block_entity_client_tick(
        &self,
        host: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::BlockEntityClientTickEvent<'_>,
    ) {
        let running = matches!(event.entity.nbt.get(&"running".into()), Some(mod_api::Tag::Byte(1)));
        let mut hums = HUMS.lock().unwrap();
        match (running, hums.get(&event.block_entity).copied()) {
            (true, None) => {
                let [x, y, z] = event.pos.map(|c| f64::from(c) + 0.5);
                let play = mod_api::SoundPlay::at("mymod:machine.hum", mod_api::SoundSource::Blocks, [x, y, z])
                    .looping(0);
                if let Some(sound) = host.play_local_sound(&play, 0) {
                    hums.insert(event.block_entity, sound);
                }
            }
            (false, Some(sound)) => {
                // A fade to 0 stops the sound when it ends.
                sound.fade(&*host, 0.0, 20);
                hums.remove(&event.block_entity);
            }
            _ => {}
        }
    }
}

struct MyMod;
#[mod_api::horizon_mod]
impl mod_api::Hooks for MyMod {
    // A machine removed, replaced or whose chunk unloaded ticks no more, and its handle no
    // longer reads: its hum stops.
    fn client_tick_post(&self, host: &mut dyn mod_api::ClientHost, _: &mut mod_api::ClientTickPostEvent) {
        HUMS.lock().unwrap().retain(|machine, sound| {
            let here = machine.position(&*host).is_some();
            if !here {
                sound.stop(&*host);
            }
            here
        });
    }
}
```

The copies are told in the order they arrived, a chunk's own in its map's order, as vanilla's client ticker
list gives. One removed, replaced by another type, changed to a block state without a client ticker or whose
chunk unloads is not told again from that tick on, so a looping sound it started plays on until something else
stops it: the example stops a hum once its machine's handle no longer reads (removal, replacement and unload
all end a handle), and a type with states that don't tick would also stop it when the state changes. Leaving
or changing levels stops every sound. Add particles from the same tick with `host.add_particle`
([hook notes](../../reference/hook-notes.md#client-ticks-hums-smoke-and-moving-parts)).

`player.open_menu(host, entity)` also returns `Result<bool, BlockEntityError>`:
`Ok(true)` means opened, `Ok(false)` means a non-provider, cancelled open or
unconnected/wrong-level player, and `Err(BlockEntityError::Stale)` means the entity
does not resolve. It never opens a replacement through an old handle.

To open a content mod menu type with no block (a backpack item's use, an entity, a command),
call `player.open_mod_menu(host, menu, title, extra, pos, replace)` (NeoForge's
`openMenu(provider, extraDataWriter)`). `menu` is the type's `MenuKey`. `extra`, at most
32 KiB, is what the client's `host.screen_extra_data()` returns while the screen is open, on
native and wasm alike. `pos`, when given, is what `screen_pos()` returns; otherwise it returns
`None`. The type's container slots show empty, and their clicks reach `native_menu_clicked`,
as `open_menu`'s do. It returns the window id, or `Ok(None)` when a mod cancelled the open.
It refuses with an `OpenMenuError`: `NotModMenu`, `PlayerUnavailable` (stale, fake, or a
vanilla client), `TooLarge`, or `Title` when the title could not cross; a cancelled or
refused open closes nothing.

The menu the player had open closes on the server first, as a close does: its `removed`, one
`player_container_close`, and the cursor stack and crafting grids back to the inventory. Then
the new menu is built, its screen opened and its contents sent, and the next tick's
`player_container_open` tells of it. `replace` is the Java `MenuProvider`'s choice
(`shouldTriggerClientSideContainerClosingOnOpen`), as for
[`open_menu`](../../concepts/capabilities/menu-slots.md#a-mods-chest-menu):
`MenuReplace::CloseFirst`, vanilla's default, first sends the client a close for the old
window, which centres the mouse again; `MenuReplace::InPlace` sends none, so the client keeps
its screen and its mouse where they are until the new screen replaces it, as when one
screen of a machine leads to the next.

## Receiving synchronized data

Implement `block_entity_data_received` on the block entity's part to react on the
client after its synchronized tag has merged. It receives a `BlockEntityRef` and
the arriving compound; reads through the handle already see the new data. This
listen notice carries `BlockEntityDataSource::Chunk` or `UpdatePacket`. A chunk
with an empty compound sends no notice. An update packet with an empty compound
sends one notice; a packet with no tag sends none. Repeated identical packets
each notify, and the event contains only the arriving fields, while handle reads
see the merged copy.
It cannot change the arriving tag. Vanilla entities, missing tags, missing client
entities and mismatched types do not call it. Server tag production uses
`block_entity_update_tag`.
