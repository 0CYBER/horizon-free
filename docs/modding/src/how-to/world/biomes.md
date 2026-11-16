# Biomes

Use biome content, region declarations and level handles to control biome generation, query biomes and edit biome cells.

| I want to | Use | Side |
|---|---|---|
| Add a biome | `data/<ns>/worldgen/biome/<name>.json` in a content mod or data pack ([content mods](../content-mods.md)) | Content |
| Add my features or spawns to vanilla's biomes | `Registrar::declare_biome_modifier` or a file in `data/<ns>/neoforge/biome_modifier/` ([content mods](../content-mods.md)) | Content |
| Give my dimension its own biomes | `DimensionSettings::biome_source`: `Fixed`, `Checkerboard`, `MultiNoise` by preset, `End`, or `Points` for an explicit climate-point layout ([dimension shape](dimension-shape.md)) | Both |
| Put my biomes among the Overworld's or the Nether's | `Registrar::declare_biome_region` with a weighted `BiomeRegion` ([biome regions](../content-mods/biome-regions.md)) | Both |
| Know the biome at a position | `LevelRef::biome_at` on a server level, `LevelRef::client_biome` on a client | Server, client |
| Find the nearest biome or tag | `LevelRef::find_closest_biome` with a `BiomeSearch` | Server |
| Change a biome | `LevelRef::set_biome`, one 4x4x4 cell, as `/fillbiome` does | Server |
| Run bone meal at a position | `LevelRef::bone_meal` | Server |
| Tint foliage, grass or water by biome | The `register_color_resolvers` hook for new colour resolvers, and the `biome_colors` hook to change a biome's own colours | Client |

Code and file modifiers support features, spawns, spawn costs, carvers, climate and effects. They apply additions,
then removals, then modifications when the world loads, after its data packs. Mods cannot choose explicit phases
or supply custom biome-modifier serializers ([content mods](../content-mods.md)).

## Adopt installed biome regions

From the client's world list, open Edit World and confirm adoption to save the installed mods' biome regions for that world. Completed chunks keep their biomes and blocks. New chunks use the adopted layout,
and chunks still being generated may finish with it. See [biome regions](../content-mods/biome-regions.md).

## Read and write limits

- `biome_at` never loads or generates a chunk: an unloaded column answers `None`.
- `find_closest_biome` follows vanilla's sampled spiral, so it is not a search through cells a mod edited, and it refuses a search whose sample count passes `BiomeSearch::MAX_SAMPLES`.
- `set_biome` refuses a client, an unknown biome and an unchanged cell, and returns false rather than an error.
