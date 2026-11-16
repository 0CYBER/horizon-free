pub trait Notes {
    #[notes = r#"## World and spawnable bounds

`LevelRef::in_world_bounds(host, pos)` and `in_spawnable_bounds(host, pos)` answer
`Level.isInWorldBounds` and `Level.isInSpawnableBounds` without loading a chunk. The first also
needs the position inside the level's build height; the second is the wider limit `/summon` and
`/tp` use (30,000,000 on x and z, 20,000,000 on y, whatever the level's height). A mod checks them
before reading or writing a far position, rather than learning it from a failed write."#]
    fn level_reads_note0();
    #[notes = r#"## Biomes, sky and floor height

`LevelRef::biome_at(host, pos)` reads an active or parked server level's biome;
`client_biome(host, pos)` reads the client's current copy. Both use the level's biome zoom,
including its hashed seed, and return `None` when the selected noise column is not loaded.
They do not generate or load chunks. A biome read may select a neighbouring column.

`find_closest_biome(host, biome, BiomeSearch::new(origin, radius, horizontal_step, vertical_step))`
uses the server generator's sampled search, as `/locate biome` does. `biome` is an id or `#tag`.
The result names the biome and its sample position. This searches generated biomes, not later
edits to resident biome cells, and does not load resident chunks. A negative radius or
nonpositive sampling step returns `None`.

`get_sky_darken(host)` shares gameplay's dimension, time and weather calculation.
`logical_height(host)` is the dimension's logical height, which can differ from build height.
`block_floor_height(host, pos)` uses normal collision shapes with an empty entity context;
the height is relative to the block, and negative infinity means no floor. An unloaded
column or an out-of-build-height position returns `None`. These three reads also work on
parked server levels and the current client level, without loading chunks or swapping dimensions."#]
    fn level_reads_note1();
    #[notes = r#"## Respawn data, maps, ticks, sky and sight

`LevelRef::respawn_data(host)` is the world spawn and the way a player appearing there faces
(`LevelData.getRespawnData`), and `set_respawn_data(host, pos, yaw, pitch)` is `/setworldspawn`'s
own write: the yaw wraps and the pitch clamps as `RespawnData.of` does, a player whose respawn
point was the old world spawn follows it, and `spawn_change` fires. Horizon keeps one world spawn,
in the Overworld, so only the Overworld's handle writes it.

`map_data(host, id)` is a map's saved data (`MapItemSavedData`): centre, scale, dimension, the
locked, tracking and unlimited flags and its 128 by 128 colour ids, from memory or from its
`.dat` without loading it for good. `set_map_data(host, id, colors)` repaints it pixel by pixel as
`setColor` does, so the map saves, redraws and reaches the players carrying or framing it. Maps
belong to the world: any current level handle reaches them. Markers are not part of either.

`block_tick(host, pos)` runs the block's scheduled tick now through the path a due tick from the
queue takes (the block's part first, then `BlockState.tick`), and `block_random_tick(host, pos)`
random-ticks the position as `tickChunk` does, lava's fluid tick included. Both draw from the
level random and send their level events exactly as the normal ticks do; neither reads or changes
the tick queue, which `schedule_block_tick` fills.

`can_see_sky(host, pos)` is `canSeeSky`: full sky light. `line_of_sight(host, from, to)` is the
clip `LivingEntity.hasLineOfSight` makes between two eyes, collider shapes and no fluids.
`lightning_target(host, pos)` is `findLightningTargetAround`: a rod, else a living entity under
open sky near the column picked with the level random, else the column's top. It strikes nothing.
Environment attributes are not here: Horizon has no general server-side attribute table to read."#]
    fn level_reads_note3();
    #[notes = r#"## Drops, fluid shapes, collisions and entity rays

`LevelRef::block_drops(host, pos, tool, entity)` is `Block.getDrops`: what the block at `pos`
would drop if `entity` broke it with `tool`, with what its block entity copies in (a shulker
box's contents, a banner's patterns, a pot's sherds). It breaks nothing and spawns nothing, and
fires no `block_drops` event; `destroy_block` with `collect` does all of that. The roll draws
from the table's random sequence or the level random, as vanilla's does, so it takes a
`ServerHost` and moves later draws.

`LevelRef::roll_loot(host, table, origin, tool, entity)` is `LootTable.getRandomItems`: one roll
of any loot table, the world's data packs merged, handed back instead of dropped, for a sieve or a
machine that turns an input into a table's roll. The context is the one `/loot … fish <table>
<pos> <tool>` gives: `origin`, `tool` for `match_tool` and enchantment bonuses, and `entity` as
`this`. The stacks come split to their stack sizes. It draws as `block_drops` does and fires no
`loot_drops` event. None for a table that isn't there.

`fluid_shape(host, pos)` is `FluidState.getShape`: no box for no fluid, else one block-local box
up to the fluid's height, a full block under the same fluid. `has_collisions(host, aabb)` asks
whether any block collision shape is in a box, with `collision_boxes`'s walk and limits and
without building the list; entities and the world border are not part of it.

`ray_trace_entities(host, from, to, margin, exclude)` lists every entity the segment passes
through, nearest first, with where it enters each box grown by `margin` and how far that is:
the test `ProjectileUtil.getEntityHitResult` makes on each candidate. Vanilla's filter is a
predicate (`canHitEntity`, `isPickable`); here it is the caller's, who takes the first hit it
accepts, for example the first living entity. A ray starting inside a box does not hit it, so
pass the shooter as `exclude` or start outside it. Candidates are the entities `entities_in`
finds, so spectators are never hit."#]
    fn level_reads_note2();
    #[notes = r#"## Fluid properties and the explicit fluid tick

The fluid queries answer for a mod's world fluid as they do for water and lava, through the same
rules the flow reads, so a custom fluid behaves like one of them without a table of its own.

Position-dependent answers are `LevelRef` reads. `fluid_tick_delay(host, fluid, pos)` is
`Fluid.getTickDelay`: water 5, lava 30 and 10 in the Nether, the empty fluid 0, a flowing form as its
source; a mod's fluid answers what it declared as its `fluid_rules` parts change it at `pos`. It reads
the level in place on the server and is `None` for a position that isn't loaded. `fluid_height(host,
pos)` is `FluidState.getHeight` (a full block under the same fluid, else `amount / 9`),
`fluid_height_for_camera` is the surface the camera's immersion test reads (a source under a block
with a sturdy underside is 1), and `fluid_can_be_replaced_with(host, pos, fluid, direction)` is
`FluidState.canBeReplacedWith`: water yields to a non-water fluid flowing down, lava to water from 4/9
of a block high, the empty fluid to anything and a mod's fluid to nothing.

World-independent facts are in the fluid service (`host.fluid_states()`): `type_properties(key)` now
also carries `explosion_resistance` (100 for a fluid, 0 for the empty one), `randomly_ticks` (lava
only) and `drip_particle` (`minecraft:dripping_water`, `minecraft:dripping_lava`, else none), and
`legacy_block(state)` is `FluidState.createLegacyBlock`, the water, lava or mod fluid block that holds
a fluid state, air for the empty fluid.

`fluid_tick(host, pos)` takes a `ServerHost` and runs the flow step of the fluid at `pos` now, the
body of a due fluid tick: it levels, spreads and schedules its next step with the same random draws,
hooks and level events. Like `block_tick` it neither reads nor changes the tick queue; `schedule_fluid_tick`
queues one."#]
    fn level_reads_note4();
    #[notes = r#"## Players, structures and chunk watchers

`LevelRef::online_players(host)` is `ServerLevel.players`: the online players in the level, parked or in
place, each once under their own UUID, also while a guest's turn is running. Fake players are not
listed, nor is a player a nested guest turn displaced, whose handle resolves again when that turn
ends. `entities_in` stays the one area query: on a server it finds players in a box too;
on a client it includes only the window's own player, not other players
(arismoko/horizon#3033).

`find_nearest_structure(host, structure, StructureSearch::new(origin, radius))` is
`ServerLevel.findNearestMapStructure` without skipping known starts, the search `/locate
structure` runs in the level's generator. `structure` is an id or `#tag`; the result names the
structure and the position `/locate` reports. `radius` counts rings of placement cells for
random-spread sets, from 0 to `StructureSearch::MAX_RADIUS` (100, `/locate`'s); strongholds' rings
are all checked whatever it is. The search builds structure starts but neither loads nor generates
chunks; a world without structures, a structure this level cannot generate, an origin past the
world's 30,000,000-block edge and a radius out of range give `None`.

`ChunkRef::watchers(host)` is `ChunkMap.getPlayers(pos, false)`: the players the chunk was sent to
and is inside the view of, from their `chunk_watch` until they move away from it. Horizon's
far-ring columns past the view distance are not watched, and fake players watch nothing."#]
    fn level_reads_note5();
}
