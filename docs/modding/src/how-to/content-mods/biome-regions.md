# Biome regions in the Overworld and the Nether

A **biome region** puts a mod's biomes among vanilla's in the Overworld or the Nether. The world is split into areas,
each area belongs to one region, and inside it the region's own climate points choose the biome. Vanilla's whole
layout is one region of weight 10 alongside your regions.

## Declaring a region

In `register`, call `declare_biome_region(path, region)` once per region. This mod adds one region to each dimension:

```rust
{{#include ../../../examples/src/biome_regions.rs:regions}}
```

- **`path`** is the region's id in the mod's namespace: `frostlands` becomes `<your mod id>:frostlands`.
- **`dimension`** is `minecraft:overworld` or `minecraft:nether`.
- **`weight`** is 1 to 1000, against vanilla's 10. A weight of 5 makes the region about half as common as vanilla's
  layout; with two mods at 1 and 3 and vanilla at 10, they own about 1/14 and 3/14 of the areas.
- **`points`** are the region's biomes by climate, the same `BiomePoint`s as a mod dimension's `Points` source
  ([dimension shape](../world/dimension-shape.md)). The point nearest to the climate at a position wins, among
  this region's points only. The biomes can be vanilla's or the mod's own (`data/<ns>/worldgen/biome/<name>.json`,
  see [content mods](../content-mods.md)).

Cover every climate the region can meet. Terrain shape never reads the biome, so an area keeps vanilla's land and sea
whatever region owns it: give oceans an ocean biome (low continentalness, here `frozen_ocean`), or land biomes end up
under water. Areas are about 512 blocks wide, and a region border changes the surface blocks and carvers, not the
height of the ground.

A malformed or repeated id, an id outside the mod's namespace, another dimension, a weight outside 1 to 1000, no
points, or a point with no biome, an interval outside -2 to 2 or the wrong way round, or an offset outside 0 to 1 is
refused. The call returns the `BiomeRegionError`, and the mods stop at startup with a message naming the mod.

## A world keeps the regions it was made with

A new world saves the installed mods' regions, their points and weights, in `data/horizon/biome_regions.dat` before its
first chunk generates. From then on that file decides, not the mods:

- **A changed region** (new points or weight) or **a new mod's region** does not reach an existing world on its own.
  The game logs the difference when the world opens and keeps generating from the saved layout.
- **Adopting the installed regions:** in the world list, select the world, open **Edit World**, choose
  **Use Installed Biome Regions**, and confirm with **Use Installed Regions**. The option appears when the
  installed regions differ from the saved layout. On a dedicated server, set `adopt_installed = true`
  under `[biome_regions]` in `server.toml`. New chunks use the installed regions: a region the world already had
  keeps its saved weight, a new one takes its declared weight, and a missing mod's region is dropped.
  Chunks that already exist keep their biomes.
- **Removing a mod** whose region the world saved is the case below.

A world made with no region mod installed has no file and generates exactly as vanilla, and so do the End and a mod's
own dimensions.

## When a mod with a region is missing

The world never opens silently without it.

- **Players (singleplayer, LAN host):** picking the world in the list opens **Biome mods are missing**, naming each
  missing mod and region. **Don't Open** goes back to the list and leaves the world as it is. **Open with Vanilla
  Biomes** records the choice in the world and opens it; it is not asked again.
- **Server owners:** the server refuses to start and names the mod and region, until `server.toml` says what to do:

  ```toml
  [biome_regions]
  missing = "vanilla"   # "refuse" (the default) does not start
  ```

With vanilla's biomes chosen, chunks that already exist keep their biomes and blocks, and new chunks in that region's
areas get vanilla's biomes. The region keeps its place and weight, so the other regions' borders do not move. If the
mod comes back, the game logs it and keeps vanilla's biomes there until the owner adopts the installed regions, since
the chunks made in between are vanilla's.
