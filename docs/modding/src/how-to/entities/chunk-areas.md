# Chunk areas for a player

A server mod can keep squares of chunks loaded, or ticking, for one player, in any level, and the engine sends them to that player's client. This is what a portal that shows another place needs, as Immersive Portals' `ChunkLoader` does. A level besides the player's own reaches only a Horizon client that can view it; it keeps that level's chunks and updates beside its own world, and nothing is drawn yet.

```rust,ignore
let nether = host.level("minecraft:the_nether").expect("the server has the Nether");
let areas = [mod_api::ChunkArea::new(
    nether,
    mod_api::ChunkPos::new(12, -3),
    4, // the radius: a square of 9 × 9 chunks
    mod_api::ChunkAreaMode::Ticking,
)];
match player.set_chunk_areas(host, &areas) {
    Ok(()) => {}
    Err(refused) => log::warn!("portal view refused: {refused:?}"),
}
// Later: withdraw them.
let _ = player.set_chunk_areas(host, &[]);
```

- **The square.** An area is the chunks within `radius` of `center` (Chebyshev distance), as `ChunkLoader(dimension, x, z, radius)`.
- **The mode.** There is no default; each area names one:
  - `Loaded`: loaded to full status, sent and kept up to date, but nothing ticks there because of the area (vanilla's `PLAYER_LOADING` ticket). A level that only `Loaded` areas name is never ticked: its chunks are generated and sent, and its time follows the server's clock.
  - `Ticking`: entity and block ticking, random ticks included, as a player's simulation ticket gives (`PLAYER_SIMULATION`). The level ticks, but only for these chunks and the ring around them.
- **Each call replaces the calling mod's areas** for that player. Other mods' areas stay. An empty slice withdraws them. A chunk no area covers any more stays on the client for 52 ticks, then is forgotten; when none of a level is left, the client drops that world.
- **Not a player.** An area is a chunk ticket. It counts as no player for natural spawning, mob caps, despawning, spawners, sleeping, raids, the dragon fight or the random-tick rules that look for a player nearby. Terrain generated for an area is saved like any other.
- **Lifetime.** Areas are never saved. They end when the player leaves.
- **The player's own level.** An area there holds its ticket, but its chunks are sent only within the player's normal view.
- **A level opened after the player logged in** (a [runtime level](../world/runtime-levels.md#players-and-clients)): its client cannot name it, so an area there is kept but nothing of the level is sent to it. When a runtime level closes, the clients that viewed it drop its world.
- **Refusals.** A call that breaks a limit changes nothing; the areas asked before stay. `ChunkAreaRefusal` names the limit and its value:
  - `NotOnline`: the player is not online on this server.
  - `RadiusTooLarge { radius, max: 32 }`.
  - `CenterOutOfRange { center, max: 1_875_000 }`: a centre more than 1,875,000 chunks (the largest world border's edge) from (0, 0) on an axis.
  - `TooManyAreas { asked, max: 32 }`: 32 areas per player, every mod's together.
  - `TooManyLevels { asked, cap }`: more levels besides the player's own than its client can view. `cap` is the client's own answer: 2 for a Horizon client, 0 for vanilla or an older Horizon.
  - `TooManyChunks { cap, level, asked, limit }`: more chunks in `level` than the server setting `cap` allows; `limit` is its value now.
  - `Unavailable`: a level handle of an earlier open of the world, or a call from outside any mod's hook (the engine keeps each mod's areas apart by its id).
- **The chunk caps** are server settings ([hosting](https://github.com/arismoko/horizon/blob/main/docs/hosting.md#other-levels-a-mod-shows-a-player-max_viewed_)). Chunks are counted as the union of every mod's areas, by mode. A `Loaded` area loads one ring past its radius, so it counts (2r + 3)² chunks; a `Ticking` area's square counts as ticking and the two rings it loads around it as loaded.

  | `ChunkCap` | Setting | Default |
  |---|---|---|
  | `LoadedPerPlayer` | `max_viewed_loaded_chunks`, per player and level | 2048, and at most |
  | `TickingPerPlayer` | `max_viewed_ticking_chunks`, per player and level | 441 |
  | `LoadedPerLevel` | `max_viewed_loaded_chunks_per_level`, every player together | 16384 |
  | `TickingPerLevel` | `max_viewed_ticking_chunks_per_level`, every player together | 3528 |
- **Cost.** With no area on the server, a level pass costs one integer compare.
