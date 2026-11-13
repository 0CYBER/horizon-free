# Natural spawning on the parallel tick

`HORIZON_PARALLEL_TICK` (`[engine] parallel_tick`, [hosting](hosting.md#parallel-tick-parallel_tick)) shares
`ServerChunkCache.tickChunks`' spawning pass among the tick's helper threads: propose in parallel, commit serially.
With it off the pass is vanilla's loop, draw for draw, and the parity harnesses pin it there.

## Why

On the official SMP (2026-11-13, 50 players, 4 cores) `spawn_natural` took 16.6 ms of a 90 ms tick, 15 ms of it in the
attempts: about 14,000 attempts a tick for about 5 spawns. Nearly all of that is read-only checks (the block and its
light, the biome's list, the distance to the nearest player, `isValidSpawnPostitionForType`), the work of #15885's
per-attempt cost aside.

## What ties the chunks together

1. **The level's random.** The chunk shuffle and every attempt draw from one stream, so a chunk's draws wait on all before it.
2. **The caps.** A spawn raises the counts of the players near it (`after_spawn`, `add_local`), which a later chunk near
   them checks in `can_spawn_local`; spawn charges add up too. The global cap is checked once, before the loop.
3. **The new mobs' boxes.** A spawn must not overlap a mob spawned earlier in the pass (`pending`, and `spawned` inside
   one pack).

## The pass

1. The server thread builds the candidates, the `SpawnState` and the categories, and shuffles the chunks on the level's
   random, as the serial pass does.
2. **Propose.** The chunks go to the lanes in parts. A chunk runs `spawn_category_for_chunk` for each category with a
   random seeded from its position and the game time (splitmix64), against a copy of the state as the pass started
   (`Lane`: the copy is made again after a chunk that proposed something, so what a lane did before never shows).
   It makes the mob (`finalizeSpawn`, from the mob's own random, which is seeded from its position) and returns the
   proposals of the chunk by pack, a pack being one category's start: its members, each with its mob and box.
3. **Commit.** The server thread walks the proposals in the shuffled chunk order. For a pack it checks
   `can_spawn_local` again for its category and chunk; for each member, `can_spawn` (the spawn charge) and, for the
   species that look for a mob in the way, the overlap with the boxes accepted so far; then it counts it
   (`after_spawn`) and queues the mob for `add_spawned`. From the first rejected member on, the rest of that pack is dropped:
   a later member stands on the earlier ones having spawned.

Caps only rise during a pass, so a chunk that was blocked at the start stays blocked, and the commit checks catch the
rest. A chunk therefore proposes a little more than the caps allow when two chunks near one player each fill the last
places, and the commit takes the first in the pass's order, as the serial pass would.

## What differs from vanilla, and what does not

The result has vanilla's odds and rules but not its draws: each chunk draws from its own random, not the level's, so a
seed does not give the mobs a vanilla level would (the same departure as the villagers' own random,
[mob-islands.md](mob-islands.md#villagers-draw-their-own-random)). The level's random is drawn from for the shuffle only,
so it is advanced differently than by the serial pass. The result is the same for any number of lanes: a chunk's random
and the state it proposes against do not depend on the lane that takes it, and the commit is in the shuffled order.

## When it stays serial

- the parallel tick is off, or there is one lane (one core);
- fewer than 512 candidate chunks (about two players' worth, a player's square is 289): a pass over fewer costs about
  half a millisecond, which handing out the work and building a level for each part is close to;
- a native mod listens to spawns (its hooks run on the server thread only; the mob islands do the same);
- `HORIZON_TEST_SEEDS` replays a seed sequence per thread (`random_source::repeating_per_thread`).

`verify` (`HORIZON_PARALLEL_TICK=verify`) cannot check this phase against the serial one: the draws differ by design.

## The level a lane reads

`GameLevel` (`game/natural_spawning.rs`) is `Level<'g, H>`: `H` is the `&Game` of the serial pass (it answers the mods
and the scan of the mobs) or `NoGame` in a lane. What a level reads of the game besides its `MobWorld` is in
`SpawnHost`, plain shared borrows. A lane's level has its own `pending` boxes and lazy caches; the grid of the mobs
and of the players is built once for all lanes. The `MobWorld` of a lane comes from the same recipe the mob islands use
(`prep::hookless`).

## Tests

- `the_spawns_are_the_same_in_any_number_of_lanes`: one and three lanes give the same mobs on one world.
- `the_second_of_two_proposals_over_a_players_cap_is_dropped`: the commit's per-player cap check.
