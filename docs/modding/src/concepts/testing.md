# Testing a mod

Enable the `testing` feature on your `horizon-mod-api` test dependency. Then
`testing::with_mods` takes an owned `Mods` list and runs a closure with it installed on the current thread.
It restores the previous list even if the closure panics. The test allocation is retained because dispatch exposes
a static slice. Call `dispatch_<hook>(&mut host, args…)` inside the closure to build the event as the game does.

`testing::FakeHost` is a server and client host built from one small fake per service, so you can test without
the game: `registries` (item reports, items with slots of their own, the loaded mods), `players` (who is online,
their inventories by slot, what you give to them and show on their action bar), `level` (blocks, block entities
with item slots, fluid tanks and energy storage as the game answers them, and entities), `commands` and
`messages` (what ran and what was said), and `journal`, which keeps the players and the level on each transaction
and puts them back when it aborts. The fake has no client backend, so client handle writes refuse.

Set up the world through those fields, run your code with `&mut host`, then inspect
the fakes. This example moves three items between two containers:

```rust,ignore
use mod_api::testing::FakeHost;
use mod_api::prelude::ItemsHost;
use mod_api::{HostTransaction, ITEMS, ItemResource, ItemStack, ServerHost, move_resources};

let mut host = FakeHost::new();
let level = host.level.level();
host.level.block_entity([0, 64, 0]).with_items(1);
host.level.block_entity([1, 64, 0]).with_items(1);
let stone_id = mod_api::Identifier::parse("minecraft:stone").unwrap();
host.registries.resolve_items([stone_id.clone()]).unwrap();
let stone = ItemStack::new(host.item_key(&stone_id).unwrap(), 5).unwrap();
host.level.block_entity([0, 64, 0]).items.as_mut().unwrap().slots[0] = Some(stone);

let server: &mut dyn ServerHost = &mut host;
let from = level.capability(server, &ITEMS, [0, 64, 0], None).unwrap();
let into = level.capability(server, &ITEMS, [1, 64, 0], None).unwrap();
let moved = server.transaction(|host, tx| {
    move_resources(host, &from, &into, |_: &ItemResource| true, 3, tx)
});
assert_eq!(moved.ok(), Some(3));
let into = host.level.block_entity_at([1, 64, 0]).unwrap().items.as_ref().unwrap();
assert_eq!(into.count("minecraft:stone"), 3);
```

Sounds and game events a hook plays in the level are kept in `host.sounds` and `host.game_events`, in order and
outside the journal, as the game sends them at once. Fluids a test uses are resolved with
`host.registries.resolve_fluids([...])`, and `host.registries.fluid_sounds(fluid, fill, empty)` gives one the
bucket sounds `fluid_pickup_sound` and `fluid_empty_sound` answer.

A fake's constructors do not install or resolve entity types. Declare its entities with
`host.level.entity(id, kind)`, then call `host.resolve_entity_types().unwrap()` explicitly.
This freezes the current load's table with requested mod keys, the player type and every kind in
that fake level. After `host.join(1, 10)` and this explicit setup,
`host.player(1).unwrap().entity().kind(&host).unwrap().id(&host)` is
`"minecraft:player"` (with `PlayersHost` in scope). Existing kinds may be instantiated after
resolution; inserting a new undeclared kind panics at the fake setup boundary. Repeated resolution
checks the frozen table rather than extending it. A new load requires a new part installation.

An unsupported host method panics with its name, so an unexpected call fails
loudly. You can write your own double for behavior the fakes do not model,
implementing the services your hook calls; `testing::FakeJournal` gives such a host the same transaction journal over its own state.
