# Hooks and the Host

A **hook** is one moment in the game: a tick, a block broken, a player joining. Your mod implements the hooks it
needs as methods of `mod_api::Hooks`, and the game calls each subscribed mod with the moment's **event** and a
**host**, its typed view of the game. A moment that belongs to one object is a method of a [Part](parts.md)
instead, called only for the objects your mod attached it to.

![The game calls every mod that implements a hook, and a Part only for its own objects. Both receive an event and
reach the game through the Host, its handles, capabilities and attachments.](../images/hooks-parts.svg)

## Dispatch

Mods run in order of their listener priority, highest first, then in dependency order: a mod after the mods it
depends on. When a mod cancels an event, the mods after it are skipped, unless they receive cancelled events
([Listener priority and cancellation](#listener-priority-and-cancellation)).

What a hook's return value means depends on its kind:

- **Listen** hooks are notices: every subscribed mod is called.
- **Cancellable** hooks return `Flow`: `Flow::Cancel` stops the game's action and skips later mods.
- **Replacement** hooks return an `Option`: the first mod to answer `Some` wins.
- **Collection** hooks call every mod, each adding to a list in the event.

A hook with no receiver gets no `Host`.

## Explosion membership

`explosion_detonate` version 2 exposes mutable `blocks` and `entities` lists
before an explosion applies its effects. The initial entity query excludes the
exploding source. Remove entries to spare them, or add live handles to make them
eligible; additions still obey normal distance, exposure and immunity checks.
Stale or invalid handles are dropped and repeated IDs are visited once. A mod
can explicitly add a valid source handle back; per-kind immunity still applies.

The final list controls membership, not global effect ordering. Its order is
used for ordinary storage targets. Crystals/dragon and players retain their
earlier separate passes, while frames and other decorations run last in storage
order. Block positions are shuffled before destruction. An empty entity list
spares direct entity effects, not indirect consequences of destroying support
blocks. With no subscriber the game does not build a target list.

## Saving and game events

To save from inside a hook, use `level.request_save(host)` so the save runs at the next settled
start-of-tick boundary. A synchronous `save-all` while an entity pass or held-out row is borrowed
fails with vanilla's `commands.save.failed` feedback and a failed command result, without saving.

To emit a vibration-producing game event, use `level.game_event(host, event, at, context)`.
`GameEventContext` carries an optional source entity and affected block state; see
[game events with context](../how-to/world/game-events.md) for listener semantics and refusals.

## Listener priority and cancellation

`#[listener]` on a hook method sets both, as NeoForge's `EventPriority` and `receiveCanceled` do. Two mods, each
its own crate:

```rust,ignore
#[mod_api::horizon_mod]
impl Hooks for Protection {
    // Runs before mods at the default priority, 0; higher runs first.
    #[listener(priority = 100)]
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        if self.protected(host, event.pos) { Flow::Cancel } else { Flow::Continue }
    }
}

#[mod_api::horizon_mod]
impl Hooks for Override {
    // Receives cancelled events and may clear cancellation.
    #[listener(priority = -100, receive_cancelled)]
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        if event.cancelled && self.allowed(host, event.pos) {
            event.cancelled = false;
        }
        Flow::Continue
    }
}
```

`priority` is an `i32`, 0 when omitted; mods of equal priority keep dependency order, as `[[inject]]`
priorities do ([Injections](injections.md#ordering)). Each hook's order is fixed once, when the mods are
installed. Every cancellable hook's event has a `cancelled` field: a mod without `receive_cancelled` always sees
`false`, since it is not called once the event is cancelled. A mod that receives cancelled events and sets it back to
`false` lets the game's behaviour run, and the mods after it are told again. An event the game hands over already
cancelled reaches only mods that receive cancelled events. `receive_cancelled` on a hook without a cancel does not
compile, and `#[listener]` is for a mod's hooks only: a part is called for its own thing, in no order.

Native and wasm mods use the same listener ordering and cancellation rules. Wasm subscription records carry the hook name, signed priority, and receive-cancelled flag through the shared-memory Wire boundary at load time. The event's `cancelled` field crosses with its other writable fields.

## Events and shared state

Each hook method takes its host, where it has one, and one event:

```rust,ignore
fn living_incoming_damage(
    &self,
    _level: &mut dyn LevelRandomHost,
    event: &mut LivingIncomingDamageEvent<'_>,
) -> Flow {
    if event.kind == "minecraft:pig" {
        let halved = event.container.new_damage() * 0.5;
        event.container.set_new_damage(halved);
    }
    Flow::Continue
}
```

The event is a generated struct named after the catalog hook, in CamelCase plus `Event`, with
the hook point's arguments as public fields: values as values and borrowed ones as references (`event.kind`). A
number or `bool` a mod may change is a plain field, written back once every mod has run, even after a `Cancel`;
anything else a mod may change, a game struct or a list, is a `&mut` field. A cancellation or replacement is
the return value; a collection hook's list is a field (`event.lines`). One event goes through every mod, so each
sees the changes before it. To exercise dispatch behavior, use the hook's dispatch function
([Testing a mod](testing.md)).

The generator validates edited stack-list outputs inside each listener's owning scope. A refused stack
output retains the immediately preceding accepted value, not the event's initial value. Scalar
outputs and cancellation still commit; later listeners see the accepted stacks and run independently
under the usual cancellation rules. Each refused output logs one warning naming the mod, hook,
output and conversion error. This policy is identical for native and wasm listeners.

Hook methods take `&self` and require `Send + Sync`. The process-wide mod list is installed once, like mixin module statics. One mod instance serves both
the window's game and the server-thread game. A hook can also fire again inside
the same mod: for example, `LevelRef::set_block` fires `block_changed`. Mods keep mutable
state with interior mutability so these calls can share the instance safely. Data-load
hooks can run on any loading thread; item default changes must be identical on every side.

`Hooks` contains global hooks. Scoped hooks belong to `BlockPart`, `BlockEntityPart`,
`ItemPart`, `MenuPart` or `FluidPart` and run for the registered object.

## Level hosts

Level hooks use access-specific hosts rather than `Host`. `LevelHost` provides
`block_state` through `LevelAccess::block_state`, client-side detection and
`state_with_property`. `state_string(state)` serializes the full name and properties, or returns `None`
for an unknown state; `min_y()` and `height()` read the level's build range. It also reads light and loaded state, through the game's own queries:
`brightness(LightLayer::Block | Sky, pos)` (`getBrightness`, the stored 0 to 15),
`raw_brightness(pos, sky_darken)` (`getRawBrightness`, which `CropBlock.randomTick` asks with 0)
and `is_loaded(pos)` (`Level.isLoaded`: inside the build height and in a loaded chunk),
`is_raining_at(pos)` (`Level.isRainingAt`, false where `pos` is not loaded) and `enabled_features()`
(`LevelReader.enabledFeatures`, a borrowed `FeatureFlags<'_>`). These queries do not allocate.
None of them loads a chunk. Light above the build height reads as full sky and below it as
none, and a chunk that isn't loaded reads as the game's query does (full sky, no block light),
so check `is_loaded` first. It also answers tag and fluid questions without a `Host`: `block_in_tag(block_key, tag)`,
`fluid_state(pos)` (`getFluidState`, `None` where the block read is),
`state_fluid_state(state)` and `fluid_in_tag(fluid_name, tag)`. Fluid reads return `FluidState` with a
`FluidKey`, amount, falling flag and engine-derived source flag; `is_empty()` and `is_source()` report
those states without a registry lookup. `block_state_id(name)` parses a block and properties, and `block_state_default(block)` gives a block's default state, as on `Host`; both return `None` for invalid input or unavailable context. Tag reads are `None` when authoritative tags are unavailable, including on a client before its server's tags are applied. For item membership and member lists, use `Host::tags()`: resolve a `TagKey` with `Tags::key`, then call `Tags::is_in_tag`, `Tags::items` or `Tags::fluids`. Membership reads return `None` on clients until server-sent tags are applied, which `tags_updated` announces; they never substitute local resource-pack tags.
`default_components_bound` precedes `tags_updated` after item defaults are published and readable.
Both notices carry `TagsUpdateCause`: `ServerDataLoad { reload }` for the server's initial data
load or reload, and `ClientPacketReceived { integrated_server }` for a client's applied tags.
`should_update_static_data()` is true on the server and a remote client, but false on an
integrated client, whose server already updated the shared static data. Installing client tags
announces the bound defaults; it does not rebind those defaults.

`LevelHost` has no random access: a draw from a read-only hook inside a level write would shift the level's random sequence away from vanilla's order. A hook that may draw is given a `LevelRandomHost`.
`LevelRandomHost` provides vanilla's `Level.random` so
mod draws come in vanilla's order. `LevelTicksHost` adds scheduling through
`schedule_block_tick` and `schedule_fluid_tick`, while
`LevelWriteHost` adds random access, `set_block`, `set_block_property`, `play_sound` and
`send_particles` and `level_event` (`levelEvent`, queued in order with the write's own events).
Outside a write, a `LevelRef` plays level events, and their global form for every player on the server
([Playing level events](../how-to/world/level-events.md)).
`set_block(pos, state, flags)` is `Level.setBlock` on a whole state, as `LevelRef::set_block` takes it: it
removes the old block's block entity and creates the new block's in the write (`LevelChunk.setBlockState`), runs
the neighbour and shape updates `flags` ask for, and queues the write's level events in order. A mod block entity
it creates starts with the data the `block_entity` hook gives, which is fired in the write and receives the level
(`LevelSide`, read-only: vanilla's `newBlockEntity` gets no level), as the `Host`'s `native_set_block` fires it. It returns true when the block
changed, and false off an authoritative server, for a position that is not loaded or outside the build height,
an invalid state, or the state the block already has. It never loads a chunk. The game owns
the tick queue; mods do not pass it back to these helpers.

## Drawing

Frame drawing through `DrawContext`, `BlockEntityDrawContext` and `WorldDrawContext` stays
in `mod_api`. Raw GPU access is native-only, unstable and outside that API, in
`mod_render::render` (`horizon-mod-render`). Replace its frame functions with native HEAD mixins compiled against the prebuilt game.
`render_needs` collects requests; `world_drawn` runs before the hand and HUD, and
`hand_drawn` runs after the hand, on the render thread. Use API draw contexts for API-typed drawing.

## Hooks and injections

A hook is not an injection point: an `[[inject]]` never runs inside a hook's dispatch. A `head` injection that
cancels the game function which calls a hook keeps that hook from firing at all
([Injections](injections.md)).
