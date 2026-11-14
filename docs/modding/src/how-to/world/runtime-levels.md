# Runtime levels

A mod opens a level of its own while the server runs and closes it again: an instanced dungeon, a
minigame arena, a pocket dimension per player. `host.runtime_levels()` has `open`, `unload`, `delete`
and `is_open`, the same on a native and a wasm mod.

```rust,ignore
fn run_mod_command(&self, host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
    let level = RuntimeLevel::new("mymod:pocket", RuntimeGenerator::Mod("mymod:arena".into()), 4242);
    match host.runtime_levels().open("mymod:instance/ab12", level) {
        Ok(handle) => { /* the level opens after this tick's levels have ticked */ }
        Err(RuntimeLevelError::CapReached) => { /* the server keeps no more open */ }
        Err(_) => {}
    }
    Flow::Cancel
}
```

## What a level is made of

- **The id** is `namespace:path` in a namespace the mod owns (its own id, or one its manifest
  lists). The folder is `dimensions/<namespace>/<path>/` of the world, derived from the id and
  nothing else, so an id that is not a folder name (`mymod:../x`, an empty segment) is refused
  with `InvalidId`, and one in another mod's namespace with `OutsideNamespace`. A vanilla level, a
  level a mod registered at startup and a level a data pack gave are `ReservedId`.
- **`dimension_type`** is a type known since startup: a vanilla type (`minecraft:overworld`) or the
  type of a level a mod registered with [`register_dimension`](dimension-shape.md#registering-a-dimension)
  (`mymod:pocket`). Nothing new crosses the network, so every client already has it. Any other
  name is `UnknownDimensionType`; a data pack's own dimension types are not offered yet.
- **`generator`** is `RuntimeGenerator::Flat` (bedrock, two dirt, a grass block, over plains),
  `Void` (air), or `Mod(name)`, a chunk generator a native mod registered
  ([`register_chunk_generator`](dimension-shape.md#chunk-generators)). A wasm mod can name a
  generator a native mod installed; one nobody installed is `UnknownGenerator`. A noise
  generator is `NoiseUnsupported` for now.
- **`seed`** is the level's own. It reaches the mod generator (`ChunkAccess::seed`), the level's block-tick
  and level randoms and the hashed seed clients are sent. It does not change structures or
  biomes. `ChunkAccess::level` tells a generator which level it fills, so one generator serves
  every instance and looks its plan up by id.

## When it happens

`open`, `unload` and `delete` check the request when they are called and return the refusal at once. The
change itself waits: the server applies the queue after its levels have ticked in the server
tick, never inside a level's tick, so no hook sees a level appear or vanish under it. Until then
`is_open` is false after an `open` and true after an `unload` or a `delete`. A request made while the queue is
applied waits for the next tick. `level_load` and `level_unload` fire as for any level the game
opens or closes, once each.

`open` returns a `LevelRef` that names this open of the level. A handle from before an `unload`
never names a later open of the same id: it reads as stale, like a handle from an earlier world.
A folder that exists under the id is reopened as it was saved, a persistent level after an
unload or a restart. Nothing reopens a runtime level by itself at the next start: the mod
reopens what it keeps, and a player saved in a level that was not reopened keeps the usual
fallback.

Success means queue admission, not apply completion. Apply failures are logged with the
requesting mod and level id. A failed open rolls back publication and frees the id for retry;
a failed unload leaves the level open for another unload request. Observe `is_open` after
the queue runs rather than treating the returned result as completion.

An explicitly opened runtime level fires `level_load` while parked: `event.dimension` names
the opened level, whereas the Host still names the current level.

## Closing

`unload(id)` saves the level and drops it, keeping the folder. It refuses with `PlayersInside`
while a player, or an arrival that has not landed, is in the level, both when it is called and
when it is applied: move the players out first with `entity.teleport_to_level`. `NotOpen` is the
answer for a level that is not open (or still waiting to open). The idle close the game gives
every level without players applies too, as to any other level: a runtime level nobody is in and
no forced chunk holds is saved and dropped from memory after 300 ticks (`level_unload` fires), but
it stays open: `is_open` still says true, and the level opens again from its folder, with
`level_load`, when a player or a mod enters it. Only `unload` or `delete` closes it for good.

## Deleting

`delete(id)` removes a level for good: its folder `dimensions/<namespace>/<path>/` and the
structure references the game keeps beside the world for it
(`data/horizon_structure_references_<namespace>_<path>.json`, kept if another id would share that
file and its level exists: `a_b:c` and `a:b_c` do), then the folders those leave empty. It works for the levels of the calling mod's namespace only, with the same refusals as
`unload` (`InvalidId`, `OutsideNamespace`, `ReservedId`), and the folder it removes comes from
the validated id and nothing else.

- A level that is **open** is unloaded first, as by `unload` (saved, `level_unload` fires), and
  its files are removed in the same pass. It refuses with `PlayersInside` while a player or an
  arrival that has not landed is in the level, when called and again when applied: move the
  players out first. `is_open` stays true until the delete is applied, which closes it.
- A level that is **not open** is only files, the leftovers of a crash or of an earlier
  `unload`. They are removed, and a level with no files is not an error. Nothing fires, since
  nothing was open.
- A level still **waiting to open** refuses with `AlreadyOpen`: its `open` comes first.
- A level with a **level below it** refuses with `LevelsBelow`, when called and again when
  applied. Folders nest: `mymod:instance/ab` lives inside the folder of `mymod:instance`, so
  removing the parent would remove the child, which may be open, with players or a save waiting.
  A level is below when its id continues the id with `/` and it is open, waiting, a level of the
  world, or has a folder under the level's own. A folder under it is any folder other than
  `region`, `entities`, `poi` and `data`, one of the first three holding a folder, or a folder in
  `data` with a level's shape somewhere below it (a folder holding `region`, `entities` or `poi`,
  or a link); the other files a mod keeps in `data` are not in the way. A folder that cannot be
  read counts as a level below. Delete the level below first.

The removal happens on the server thread, in the pass that applies the queue, after the level
has been saved and dropped, so no save is running against the files. A request in the queue is
applied in the order it was made. A reopen therefore waits for the removal: an `open` of a level
that is open and being deleted is `AlreadyOpen` until the delete is applied; an `open` of a level
that was not open, made after the `delete`, is applied after it and starts from nothing. A save a
mod asked for with `level_request_save` is dropped with the level.

A link (symbolic link, junction) on the way to the folder under `dimensions`, or to the
references file under `data`, is never followed: the removal fails before anything is removed.

If the removal fails (a link, a file another program holds), the failure is logged with the mod
and the id, and what happened to the level depends on how far the delete got. A level that could
not be closed, or has a level below it, stays open and `is_open` stays true; delete it again.
A level that closed but whose files could not all be removed stays **closed**: `is_open` is
false, its handle is stale, `open` of the id answers `Removing`, and an `open` already waiting
behind the delete is dropped with a log line instead of starting from the files that are left.
A second `delete` of the id finishes the removal, and only then can it be opened.

A handle from before the delete reads as stale, as after an `unload`.

## Players and clients

A player goes into a runtime level and out of it as into any level, with
`entity.teleport_to_level`. Its client, vanilla or Horizon, learns the level on the way in: the
Respawn names it and its dimension type, which the client has had since it joined. On the way out
a Horizon client forgets a level the server opened after it logged in, so a long session through
many instances keeps none of them. A level opened again under the same id after a `delete`
reaches the client afresh, with nothing of the old one.

A client learns at login which levels the server has, and names the levels it views by their place
in that list. A runtime level opened after a player logged in is not in that player's list:
[chunk areas](../entities/chunk-areas.md) there are kept, but nothing of the level is sent to that
player's client, which sees it only by going in. A player who logged in after the level opened
views it as any other level. When a runtime level closes, by `unload` or `delete`, every client
that viewed it drops its world of it; the areas stay, and show the level again once it is open
again, to the clients that can name it.

## The cap

A server keeps at most 64 runtime levels open or waiting, whichever mods opened them: the
65th `open` is `CapReached`. A server sets another number with `horizon-max-runtime-levels` in
`server.properties`; a server run by `horizon-server` sets it under `[properties]` in `server.toml`
([hosting](https://github.com/arismoko/horizon/blob/main/docs/hosting.md)). An unloaded or deleted level frees its place when the request is applied; the idle close does not free it.

Every refusal is also logged with the name of the mod that asked.
