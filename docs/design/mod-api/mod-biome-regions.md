# Design: a mod's biomes inside the Overworld and Nether

Scope 2 of [arismoko/horizon#2554](https://github.com/arismoko/horizon/issues/2554): how a mod's biomes generate among vanilla's in the Overworld and the Nether. The design below was ruled on in [arismoko/horizon#2554](https://github.com/arismoko/horizon/issues/2554#issuecomment-6803604498) and built in the stages listed at the end. One part is not built yet and is marked so. Scope 1, a mod's own dimension with its own climate points (`BiomeSourceKind::Points`), is separate and does not need this note.

## Starting points

Ari's answer on arismoko/horizon#2554 set two starting points:

- **Regions with weights.** Each mod lays out its biomes in its own region. A hidden extra parameter decides which region owns an area, and vanilla's layout is one region among them. Ordering between mods is settled by the regions, not by competing for the same climate points.
- **The mix is fixed when the world is created and saved with the world.** Terrain in an existing world changes only by a recorded, per-world choice.

The same answer says NeoForge has no API for this: its biome modifiers change biomes that already generate and cannot add a biome to a dimension's layout, and the region mechanism is a third-party library. So there is no NeoForge contract to mirror. This note did not read that library's source and claims nothing about its behaviour.

## Horizon today

| Fact | Where |
| --- | --- |
| A level's biome source is one of fixed, checkerboard, multi-noise by preset, the End, or (scope 1) a mod's own points. | `BiomeSourceKind` in `crates/horizon-mod-vocab/src/world/dimension.rs` |
| The game writes the level stem's `biome_source` JSON; the generator decodes and resolves it once per dimension. | `biome_source` in `game/modding/native_dimensions.rs`; `worldgen/biome_source/codec.rs`, `runtime.rs` |
| A multi-noise source is a list of `(ParameterPoint, biome)` searched for the nearest point to the six climate values and a zero offset. | `worldgen/climate/index.rs` (`ClimateIndex`, `ClimateSearch`) |
| The Overworld's and the Nether's lists are named presets read from the prepared data, not data a mod can extend. | `GeneratorData::load`, `MultiNoiseBiomeSource::from_quantized_json` |
| A chunk stores its biomes per section, so a chunk that already exists keeps them whatever the source says later. A saved biome the game lacks is kept by name once arismoko/horizon#3132 lands. | `decode_column` / `encode_with` in `crates/horizon-client/src/save.rs` |
| Terrain shape never reads the biome: density and the router read only climate. Surface rules and `carver_biome` do read it. | `worldgen/density.rs`, `router.rs`, `generator.rs` |
| Seeded Overworld and Nether preset output is already compared with the Java oracle. | `tools/test_biome_generation_parity.py` |

## Regions

**Region.** A region is `(owner mod, region id, weight, points)`, where `points` are the `BiomePoint`s of scope 1. Vanilla's own list is the region `minecraft:vanilla`. A mod declares one at load with `Registrar::declare_biome_region(path, region)`, where the `BiomeRegion` holds the dimension (the Overworld or the Nether), the weight and the points. The id is namespaced to the declaring mod. A malformed or duplicate id, another dimension, a weight outside 1 to 1000, an empty point list or an invalid point refuses, naming the mod. In the mod API the record is `BiomeRegion`. The engine's own `worldgen::biome_chunk::BiomeRegion` is a different type that mods never see; where both meet in one engine file, alias one at the import.

**Weights.** Integers. A mod region's weight is 1 to 1000. Vanilla's region is 10 by default, so a weight of 10 means "as common as vanilla's whole layout". A zero weight and an overflowing total are refused. A mod's declared weight is only the default written into a world's record when the world is created. After that, the record's weights are the ones generation uses, and nothing reads a weight from the mod. Players and server owners can then change them without a format change. A setting that edits them at creation is a separate follow-up.

**Order.** Regions are sorted by owner mod id, then region id, the same rule the attribute catalog uses. The same mods give the same order on every side, so the list can be saved and compared.

## Choosing a region

**Choosing a region.** Selector version 1 uses jittered cells in quart coordinates. Each cell draws jitter x, jitter z and a cumulative-weight pick, in that order, from `root.from_hash_of(salt).fork_positional().at(cx, 0, cz)`. `from_hash_of` borrows the positional root and returns a fresh source; the fork advances that derived source only, leaving vanilla's streams unchanged. Positional y is fixed at 0, so ownership is independent of height. The owner is the nearest jitter point among the 3x3 candidate cells, with x scanned inside z and the first candidate winning a tie. This is the neighbourhood definition, not a promise to find the globally nearest jitter point. Negative coordinates use floor division, and a pick at a cumulative slot end belongs to the next slot. A different neighbourhood or draw rule requires a new selector version, not a radius field. Each selected region searches only its own climate index. With no layout or no regions, today's preset source and random draws are unchanged.

With a layout, each resolver caches the 16 quart columns of its current chunk, without allocating per sample. Each first column visit evaluates nine cells and 27 bounded draws; other heights in that chunk reuse the owner. Crossing into another chunk resets that cache. Climate lookup still runs per quart. There is no tick-time change without a layout; no runtime timing has been measured for the enabled path.

**What changes at a border.** Surface material, carvers, features, decoration and structure eligibility follow biome selection. Only base density is unchanged with an unchanged router. A structure whose biome check straddles a border follows vanilla's own check at its sample position. Height-dependent biomes and old-biome blending can make chunk fill and a carver's sample differ within one region.

## The saved record

One durable world-level file, `data/horizon/biome_regions.dat`, separate from `data/minecraft/world_gen_settings.dat`:

```
{ version: 1,
  dimensions: { "minecraft:overworld": {
      selector: { version: 1, cell: 128, salt: "horizon:biome_region", vanilla_weight: 10 },
      regions: [ { owner: "my_mod", id: "my_mod:highlands", weight: 5,
                   points: [ { biome: "my_mod:highland", temperature: [min, max], …, offset } ],
                   substituted: { with: "minecraft:vanilla", missing_mod: "my_mod" } /* only after a choice */ } ] } } }
```

- **Contents.** Regions are stored in sorted order with their full points, not hashes. A new world writes the record before its first chunk is generated. Version 1 bounds the file and decompressed NBT to 32 MiB, two dimensions, 1024 regions per dimension and 16384 points per region (two million NBT nodes, depth 32). Oversized declarations or records are refused; no truncation occurs. Its point validation is pinned independently of later registration rules. The selector cell is 1 through 2^30, and its total weight fits a positive i32.
- **Absent record.** Region selection is off and every dimension keeps its existing source, custom dimensions included.
- **Malformed record.** The open is refused with the reason, and the file is left untouched on disk: no rewrite, no rename, no default record in its place.
- **Changed mods.** An installed mod whose declared regions differ from the record (points, biomes, or a region added or removed) never changes an existing world silently. The record wins, the difference is reported once by name (the record persists the reported names), and the new layout is taken only through the opt-in below.

## Missing mods and opting in

- **A saved region's mod is absent.** The world names each missing mod and region and requires a choice: do not open, or open and resolve that region through vanilla's index from then on. The region keeps its slot and weight, so the other regions' borders do not move. The choice is recorded on the region with the missing mod's id.
- **Client.** The question is asked in the existing pre-load prompt, where the backup questions are.
- **Dedicated server.** `server.toml` gets `[biome_regions] missing = "refuse" | "vanilla"`, default `"refuse"`. Without the setting the server refuses to start, naming the mod and region.
- **The mod returns.** The substitution is reversible. When the mod returns, the world says so once and keeps the substitution until the owner opts back in, since the chunks generated in between are vanilla there.
- **Opt-in.** "Use the installed regions for new chunks" is a recorded per-world choice, off by default: on a server `adopt_installed = false` under `[biome_regions]`. Adopting keeps the saved weight of every region that is still declared, takes the declared weight for a new one, and drops the regions of mods that are not installed. In single player, the world's Edit World screen shows **Use Installed Biome Regions** only while adopting would change the world's record. It asks first, saying that new chunks beside existing ones may differ in surface, decoration and structures, and then writes the choice into the world. The new layout is written durably before any chunk is generated with it. Completed chunks keep their saved biomes and blocks. A saved `Terrain` or `Features` proto chunk resumes with the currently adopted biome layout: its saved blocks, ticks, entities and post-processing are restored, but biome filling and structure lookup use the current generator. Remaining decoration uses that generator; a saved `Features` chunk does not repeat its completed decoration. The old layout is not pinned per proto chunk.

## Acceptance matrix

Each row names fixture coverage, not a passing run. The generated-column persistence test and both-server-role creation test require prepared resources; the latter is ignored by default. The seeded preset oracle and structure-border test still need the separately ordered worldgen run.

| Fixture | Expected | Covered by |
| --- | --- | --- |
| No region declared | Overworld and Nether biome output identical to today for fixed seeds (seeded preset oracle), negative coordinates and chunk edges included. | `tools/test_biome_generation_parity.py` (the seeded preset oracle, unchanged path); `horizon-world` `worldgen::biome_source::region_selector_tests::without_regions_the_source_is_vanilla_unchanged` (no record, an empty record and a never-chosen region all give vanilla's biomes over negative and positive quarts, scalar and prepared) |
| Two mods, weights 1 and 3, vanilla 10 | Over a large sample, region ownership follows the weights within a stated tolerance; the same seed gives the same map twice. | `horizon-world` `worldgen::biome_source::region_selector_tests::cells_split_by_weight` (20 000 cells, each share within 0.02 of its weight / 14); `horizon-world` `worldgen::biome_source::region_selector_tests::same_seed_and_layout_give_the_same_owners` |
| Same mods installed in a different order | Same sorted list, same map. | `horizon-mod-api` `parts::biome_regions::tests::declared_regions_install_in_mod_then_id_order` (the list); `horizon-world` `worldgen::biome_source::region_selector_tests::selection_reads_only_the_saved_weights_in_order` (the map follows the saved list only) |
| A structure whose biome check straddles a border | Placement follows vanilla's check at its sample position. | `horizon-world` `worldgen::generator::region_tests::structure_biome_checks_read_the_region_at_their_own_quart` |
| World created with mod A, mod A removed | The missing mod and region are named; do-not-open or a recorded vanilla substitution is required. Dedicated servers refuse without the setting. Existing chunks are never touched. | `horizon-client` `save::biome_regions::tests::a_missing_mods_region_refuses_the_open_naming_mod_and_region`; `horizon-client` `save::biome_regions::tests::choosing_vanilla_is_saved_and_opens_from_then_on`; `horizon-client` `title::missing_biome_regions_tests::a_missing_biome_mod_asks_before_the_world_opens` (the prompt); `horizon-client` `game::interface::dedicated::biome_regions::tests::missing_mods_refuse_and_nothing_is_adopted_unless_set` and `horizon-server-launch` `config::tests::biome_regions_refuse_missing_mods_unless_their_section_says` (the server); `horizon-world` `worldgen::biome_source::region_selector_tests::a_substituted_region_resolves_through_vanilla_and_moves_no_border`. `save::biome_regions::persistence_tests::generated_chunks_survive_restart_substitution_added_regions_and_adoption` generates and saves a column and two proto chunks, generates new terrain after each choice (no opt-in, the recorded substitution, the explicit adoption), and reopens the saved column and chunks once, at the end, checking their biomes and region-file bytes. |
| Mod A returns after a substitution | Reported once; the substitution stays until the owner opts back in. | `horizon-client` `save::biome_regions::tests::choosing_vanilla_is_saved_and_opens_from_then_on` (the return is a difference and the record is unchanged). `save::biome_regions::tests::named_reports_are_remembered_across_restarts` pins persisted changed/returned names and no rewrite on the second open. |
| Mod A changes its climate points or weight | The saved points and weights win until a recorded opt-in; the difference is reported once. | `horizon-client` `save::biome_regions::tests::changed_added_and_removed_regions_are_reported_and_the_record_wins` |
| Mod B added to an existing world, no opt-in | No change in any generated or new chunk. | `horizon-client` `save::biome_regions::tests::changed_added_and_removed_regions_are_reported_and_the_record_wins` (the record, which generation reads, is unchanged). `save::biome_regions::persistence_tests::generated_chunks_survive_restart_substitution_added_regions_and_adoption` also compares generated biomes without adoption. |
| Opt-in on an existing world | New chunks may use B's region; existing chunks unchanged; the saved list includes B afterwards. | `horizon-client` `save::biome_regions::tests::adopting_takes_installed_regions_and_keeps_saved_weights` (the saved list); the server setting in the dedicated test above. `save::biome_regions::persistence_tests::generated_chunks_survive_restart_substitution_added_regions_and_adoption` checks adopted new biomes against saved old biomes and unchanged region bytes. The client prompt tests cover Escape, failed writes with retry and changed records at confirmation. |
| Malformed record | The open is refused with the reason; the file is unchanged on disk. | `save::biome_regions::tests::a_malformed_record_is_refused_and_left_untouched`, `every_required_record_field_refuses_missing_or_wrong_types`, `saved_selector_identity_climate_and_counts_are_bounded`, `oversized_and_overbudget_nbt_refuse_before_materializing_the_record` and `literal_version_one_file_decodes_without_the_record_encoder` |
| A region with invalid points, a bad weight or a duplicate id | Refused at registration, naming the mod; no region installed. | `horizon-mod-vocab` `world::biome_region::tests::a_region_needs_a_preset_dimension_a_weight_and_valid_points`; `horizon-mod-api` `parts::biome_regions::tests::biome_region_declarations_validate_and_refuse_duplicates` |
| A wasm mod's region | Reaches the game as declared. | `horizon-mod-wasm-wire-loopback` `wire_loopback_biome_region_declaration_reaches_the_host_bit_exact` |

The same resource-gated generated-column fixture also saves and reloads both `Terrain` and `Features` proto chunks, checking that resumed biomes use the adopted layout while saved blocks and completion status remain. The agent JSON contract is covered by the ignored `game::agent::tests::region_adoption_agent_state_reports_the_actual_title_mode` fixture.

## Stages

1. Saved biome ids kept when the game lacks them (arismoko/horizon#3132).
2. `BiomeRegion`, `declare_biome_region` and validation, with unit tests and a wire round trip. No generator change. **Built.**
3. Region selection in the biome source (`ResolvedBiomeSource`), behind "no record means today's path", with the oracle, distribution, order and structure fixtures. **Built.**
4. The saved record (`crates/horizon-client/src/save/biome_regions.rs`), world create and open, the missing-mod prompt, the server settings and the opt-in, with this acceptance matrix. **Built**, with client adoption on Edit World. The ignored `game::world_loading::tests::both_server_roles_record_regions_before_world_generation` fixture checks that both roles save the Overworld and Nether regions under their preset keys and use them in generated chunks.
5. User docs ([biome regions](../../modding/src/how-to/content-mods/biome-regions.md), [hosting](../../hosting.md)), an example mod (`docs/modding/examples/src/biome_regions.rs`, one Overworld and one Nether region) and the tests named in the matrix. **Built.**
