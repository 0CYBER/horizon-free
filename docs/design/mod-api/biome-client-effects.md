# Design: per-biome client effects and mod biome registry sync

Provisional answer to [arismoko/horizon#2734](https://github.com/arismoko/horizon/issues/2734). A design note, not an implemented API; it files no issues or PRs and claims nothing. Historical leads below are not a current gap inventory: known-biome replacement is already implemented; the connection acceptance fixture remains verification work. Line numbers are at base `3be6d3ecd1` (the stage 4 snapshot branch, PR arismoko/horizon#2769) unless a branch is named.

The issue thread's starting point holds: the game already sends the world's biome registry in network order and builds the biomes a client lacks from their data. The open work is how a mod's declaration joins those tables, which changes reach a remote client, and what stays unverified.

## What the client reads today

26.3 split a biome in two. Through `python tools/rea_vanilla.py class net.minecraft.world.level.biome.BiomeSpecialEffects`, the effects record keeps only the water colour (an int), the optional foliage, dry foliage and grass colours, and the grass colour modifier. Everything else moved to the biome's `attributes` map (`Biome.getAttributes`, an `EnvironmentAttributeMap`). `EnvironmentAttributes` lists, among others, `SKY_COLOR`, `FOG_COLOR`, `WATER_FOG_COLOR`, `AMBIENT_PARTICLES`, `AMBIENT_SOUNDS`, `BACKGROUND_MUSIC` and `MUSIC_VOLUME`. So in biome JSON:

| Effect | Where in 26.3 biome JSON | Vanilla type |
| --- | --- | --- |
| water, foliage, dry foliage, grass colour, grass modifier | `effects` | `BiomeSpecialEffects` |
| sky, fog, water fog colour | `attributes."minecraft:visual/sky_color"`, `fog_color`, `water_fog_color` | attribute |
| ambient particles | `attributes."minecraft:visual/ambient_particles"` | list of `AmbientParticle(particle, probability)` |
| ambient loop, additions, mood | `attributes."minecraft:audio/ambient_sounds"` | `AmbientSounds(loop, mood, additions)` |
| music | `attributes."minecraft:audio/background_music"`, `minecraft:audio/music_volume` | `BackgroundMusic(default, creative, underwater)` |

Horizon reads the same split. `BiomeInfo::from_json` takes colours from `effects` and sky, fog and water fog from `attributes` (`crates/horizon-client-render/src/biome.rs:211-259`), keeps the whole attribute layer (`:273`), and parses ambient particles at `:112-127`. Three consumers each read their own attribute:

- Particles: `animate_tick.rs:652` uses `BiomeInfo::ambient_particles` for a pack or server biome (`pack_file(name)` is set), and a table built at load for the rest. That table (`animate_tick.rs:374-394`) reads `data/minecraft/worldgen/biome/<name>.json` from disk, so it knows only built-in biomes by file name.
- Ambient sounds: `AmbientTables::at` (`ambience.rs:90-100`) takes the biome's pack JSON when there is one, else the built-in table, else the dimension type's value.
- Music: `MusicTables::at` (`music.rs:227`) does the same for `background_music` and `music_volume`.

Vanilla 26.3, from bytecode: `ClientLevel.doAnimateTick` loops `environmentAttributes().getValue(EnvironmentAttributes.AMBIENT_PARTICLES, pos)` for a block whose collision shape is not full and spawns each particle where `canSpawn(random)` (`random.nextFloat() <= probability`). `BiomeAmbientSoundsHandler.tick` reads `AMBIENT_SOUNDS` at the player: a changed `loop` fades the old `LoopSoundInstance` out and the new one in; each `AmbientAdditionsSettings` plays when `random.nextDouble() < tickChance`; the `AmbientMoodSettings` lowers a moodiness counter by sky light or (block light - 1) / `tickDelay`, and at 1.0 plays the mood sound `blockSearchExtent` blocks around, offset by `soundPositionOffset`. `AmbientMoodSettings` fields are `soundEvent`, `tickDelay`, `blockSearchExtent`, `soundPositionOffset`; `AmbientAdditionsSettings` are `soundEvent`, `tickChance`. Horizon's `ambience.rs:15-30` mirrors them. The attributes are values the player's position resolves, so a mod needs no client code: if the biome JSON is on the client, the effect works.

The consequence for sync is that **a client plays a biome's effects only from the biome JSON it holds**. The three tables above are keyed by biome name and by `Biomes::pack_file`.

## How a biome reaches a remote client today

1. The server builds its registry in network order: `Biomes::registry_order` sorts by path, then namespace (`biome.rs:377`), the order `RegistryDataLoader` uses. `SyncData` (`lan/data.rs:483-499`) rewrites the biome entries from that table when a pack added or replaced one, flagging them `from_pack`.
2. `registry_packets` (`lan/data.rs:632-657`) sends an entry's data when it is `from_pack` or the client did not accept the vanilla pack, and omits it otherwise (`None if known_pack && !from_pack`).
3. The client fills an omitted entry from its own resources: `fill_known_entries` (`net.rs:1236`) reads `data/<ns>/worldgen/biome/<path>.json`.
4. Supplied biome definitions replace known names in place and append missing names. Historically, biomes the client lacked were built from the sent NBT: `client_biomes` (`lan/data.rs:56-76`) keeps `known` and runs `Biomes::with_packs` over the missing names only (`!known.contains(name)`). `remote/mod.rs:2599-2611` installs the table, and `:2616-2631` maps each network index to a table id by name, so the table's own order (built-ins first, new ones appended, `biome.rs:336-352`) need not equal the network order. This is also why a new biome's table id is not its network id.

The historical baseline suggested two gaps; the first is implemented:

- **Known-biome replacement is implemented.** `client_biomes` now forwards all supplied definitions and `with_packs` replaces names in place, covered by `packs_replace_in_place_and_add_after_the_builtin_biomes`. The connection acceptance fixture remains verification work. At the historical baseline, `client_biomes` filters out names the client already has, so a server that replaces `minecraft:plains` (a pack, or a mod) sends its data (`from_pack`) but the client keeps its own. The issue thread asks for exactly this acceptance fixture.
- **A modifier-changed vanilla biome is not flagged.** Biome modifiers rewrite `data/<ns>/worldgen/biome/<path>.json` in the content root (`mod_content.rs:2554`) and the server's `walk` reads the root, but the entry stays `from_pack == false`, so with the vanilla pack accepted its data is omitted and a Horizon client without the same mod fills it from its own, unmodified file. Whether the dedicated server's `walk` root is the modified one I did not trace, so this is a lead, not a finding. It matters because the MODIFY phase (below) changes colours the client tints with.

Where a content mod's brand-new biome file enters `Biomes` is also unverified. `Biomes::load` reads only the `minecraft` folder (`biome.rs:291-310`); `packed_biomes` (`world_loading.rs:590-618`) adds entries from `worldgen_overrides` and the modifiers. I found no test that a mod ships `data/<ns>/worldgen/biome/x.json` and plays it.

## Options for a mod's biome

(a) **A datapack-registry-style registry** (the stage 4 transport: `horizon:datapack_offer`, `horizon:datapack_registries`, `crates/horizon-client/src/datapack_net.rs:20-22`, `DatapackSnapshot::candidate` in `crates/horizon-mod-vocab/src/world/datapack_snapshot.rs`). It declares a typed, schema-checked registry of values. That is the right tool for a registry the mod owns; a biome is not one. The biome registry is vanilla's, its codec is vanilla's `Biome`, and a vanilla-protocol client must receive it in the configuration phase anyway (see (c)). A second transport would duplicate data the vanilla path already carries and give two ids per biome. Not recommended.

(b) **The dimension-type route (arismoko/horizon#2147).** A mod hands the engine a table once at startup (`install_mod_dimensions`, `game.rs:17637`, a `OnceLock` at `crates/horizon-world/src/worldgen/overrides.rs:79`), and `mod_dimension_types` (`lan/data.rs:187-218`) appends its entries with `from_pack = true`, so their data is always sent. Two properties carry over: declaration happens once before any world opens, and always-send avoids the known-pack hole above. One does not: dimension types go through `PACK_REGISTRIES`, biomes use the dedicated branch in `lan/data.rs:483-499` with `Biomes::with_packs`.

(c) **The vanilla registry sync.** `minecraft:worldgen/biome` is first in `SYNCHRONIZED` (`lan/data.rs:17-20`), so every client, vanilla protocol included, receives `RegistryData` for it in the configuration phase. A vanilla-protocol client that lacks a mod biome can decode it from the sent NBT (`Biome` has a `NETWORK_CODEC` in the bytecode). Its id is the entry's index in that packet, which must be the sorted order (path, then namespace) over all biomes including the mod's; chunk biome palettes use those indices (`ChunkFormat::new(..., biomes, bits)`, `remote/mod.rs:2627`). A mod biome therefore changes the network id of every vanilla biome sorted after it. That is already how a pack-added biome behaves.

**Recommendation: (c) carried by the (b) pattern.** A mod's biome is a normal registry entry, flagged `from_pack` so its data is always sent (as dimension types are), with its file declared at startup, and the client's existing `client_biomes` path builds it. No new packet, schema or negotiation. The replacement change is already implemented; remaining connection verification must exercise it. The original plan was: `client_biomes` replaces a known biome whose sent data differs from the client's own file, and modifier or MODIFY changes mark the entry `from_pack`. The effects need nothing else, because the three consumers above already take `pack_file`. A Horizon client without the mod gets the biome and plays its effects from the sent JSON; a vanilla client gets the biome and the vanilla behaviours of whatever attributes it understands. The datapack-registry transport stays for registries a mod owns.

Files to change: `crates/horizon-client/src/lan/data.rs` (`client_biomes` replacement rule; `from_pack` for modified and mod entries; a `mod_biomes` beside `mod_dimension_types` if the source is an install table), `crates/horizon-client-render/src/biome.rs` (`with_packs` is already the merge point; add a replace-known test), `crates/horizon-client/src/game/network/remote/mod.rs` (`:2599-2611`, rebuild when the sent table differs from a known one), `crates/horizon-client/src/animate_tick.rs` (`:374-394`: stop reading the disk file for non-built-in names; the `pack_file` branch already does), `crates/horizon-client/src/mod_content.rs` (carry a mod's biome files into `packed_biomes`, if that route does not exist), and the docs below.

## Which effects a mod sets, and the API

A data-file mod needs no code: it ships `data/<ns>/worldgen/biome/<path>.json` with the fields in the table above, for a new biome, and `horizon:modify_biome` for a vanilla one. Sounds and particles name events and types in the `minecraft` or the mod's own namespace, which the client already resolves.

PR arismoko/horizon#2800 (`origin/xmilon/2732-biome-modify-phase`, commit `22c8f24053`) adds the MODIFY phase for climate and colours: `horizon:modify_biome` sets `temperature`, `downfall`, `has_precipitation`, `temperature_modifier` and `effects` colours `sky_color`, `fog_color`, `water_color`, `water_fog_color`, `foliage_color`, `grass_color`; the code form is `BiomeModifierOperation::ModifyBiome` in `crates/horizon-mod-vocab/src/world/biome_modifier.rs` on that branch, checked by `BiomeModifier::check` (colour at most 0xFFFFFF). Its docs (`how-to/content-mods.md`) say sky, fog and water fog are written to their `minecraft:visual/...` attributes. What is left:

- **Ambient sounds, particles and music cannot be modified.** `ModifyBiome` has no field for them. Proposed additions as `Option` fields in the same shape: `ambient_particles: Option<Vec<(String, f32)>>`, `ambient_loop: Option<String>`, `ambient_additions: Option<Vec<(String, f64)>>`, `ambient_mood: Option<AmbientMood { sound, tick_delay, block_search_extent, offset }>`, `background_music` and `music_volume`. In the file they are the vanilla attribute objects under `attributes`, replacing the biome's whole value (these attributes are not merged field by field); a set field means replace, an absent one leaves the value. Validation as PR arismoko/horizon#2800: out-of-range or wrong-type values skip the modifier with a warning naming mod and file.
- The JSON keys of the audio objects (`loop`, `mood.sound`, `mood.tick_delay`, `mood.block_search_extent`, `mood.offset`, `additions[].sound`, `additions[].tick_chance`) are derived from the bytecode accessors above, not from a prepared biome file; confirm them against one before code.
- **A new-biome declaration in code** (no file) is not designed here; reuse the declaration style of `declare_biome_modifier` if wanted.
- NeoForge: its public biome modifier page ([docs.neoforged.net/docs/worldgen/biomemodifier](https://docs.neoforged.net/docs/worldgen/biomemodifier/), labelled 26.1) documents the five phases including MODIFY ("single values such as climate and colors") and no setter names for sounds, particles, music or colours. The 26.3 per-biome surface is **unconfirmed**; this note does not claim counterpart equivalence.

## Staged plan

1. **Fixtures first** (no behaviour change): add the tests named below against the current code to learn which already pass.
2. Replacement rule in `client_biomes` and `with_packs`: implemented, with an in-place replacement unit test. Retain the connection acceptance fixture and verify `from_pack` for modified entries separately.
3. A mod's new biome file reaches `packed_biomes` and the server registry; effects play on a remote client.
   Status: implemented. `Biomes::load` appends the content root's non-`minecraft` biomes from `worldgen/biomes.json` after the built-ins in registry order (`biome::mod_biome_files`), `pack_file` answers for them, and `SyncData` adds them to the biome registry with `from_pack` set, so a vanilla-pack client receives their data. Not covered: a dimension generating the biome (its biome source) and reload while connected.
4. The ambient and music fields of the MODIFY phase, file and code form, with docs.
5. Gameplay-side readers (any server logic that reads these attributes) are out of scope.

## Acceptance fixtures

Expected values are proposals, not results.

| Fixture | Expected |
| --- | --- |
| Server pack replaces `minecraft:plains` with `effects.water_color` 0x112233 and ambient particle `minecraft:ash` at 0.25 | client table: id of `minecraft:plains` unchanged, `water` = 0x112233, `ambient_particles` = [(ash, 0.25)] |
| Mod adds `huts:dunes` beside built-ins | network order puts it by path then namespace; the client table has it appended after built-ins, `id("huts:dunes")` resolves by name; a chunk column keeps its biome after the join |
| `ambient_sounds` loop `minecraft:ambient.warped_forest.loop`, additions chance 0.0111, mood tick delay 6000, search extent 8, offset 2.0 on the new biome | `AmbientTables::at` returns the same values |
| `horizon:modify_biome` changes a vanilla biome's `grass_color`; remote Horizon client without the mod | after the fix, tint equals the server's; today it equals the client's own file |
| Two mods both declare biome `a:x` | startup refusal naming both mods (as dimensions) |
| Reconfiguration to a server with fewer biomes | a stale column plays plains and never indexes past the table (existing test `a_reconfiguring_client_plays_menu_music_and_never_indexes_past_the_biomes`, `remote/mod.rs:16003`) |

## Open questions

- Is replacing a known biome's data on the client allowed for every server, or only when the join negotiated a Horizon mod list? Recommendation: always, since the data arrives only with `from_pack` and the client already trusts the server for dimension types.
- Does a vanilla-protocol client accept an unknown-namespace biome id? The wire format allows it; I could not test it.
- Reload while connected: vanilla reconfigures to change registries. Which Horizon path does a mod's biome change use on a live join?

## What I could not verify

- The names and signatures of NeoForge 26.3's per-biome setters. What the public record does say (the 26.3 changelog at maven.neoforged.net, `neoforge-26.3.0.102-changelog.txt`, and the descriptions of the pull requests it names on github.com/neoforged/NeoForge; no source or patches were read): the docs site has no 26.3 version (its newest is 26.1, which is the page cited above) and the 26.3 Javadoc host returned 503; `26.3.0.19-beta` "[26.3] Add ability to remove color overrides in BiomeSpecialEffectsBuilder" (arismoko/horizon#3528, a modifier can unset a colour override as well as set one; the PR text gives no field list); `26.3.0.24-beta` "Provide registries access to BiomeModifier and StructureModifier" (arismoko/horizon#3546: `modify` receives the server's `RegistryAccess`); `NoneBiomeModifier` was removed in favour of data loading conditions (arismoko/horizon#3165, described in arismoko/horizon#3553), so `neoforge:none` is a 1.21-era form that Horizon still reads for older mods. Nothing in the changelog names a per-biome sound, particle or music setter, so those stay Horizon's own `horizon:modify_biome` fields and no counterpart is claimed.
- Vanilla client behaviour beyond the methods named (`doAnimateTick`, `BiomeAmbientSoundsHandler.tick`); music selection is cited from Horizon's own `music.rs`, not re-read from bytecode.
- That `walk` in `SyncData::load` reads the modified content root on a dedicated server, and where a mod's brand-new biome file enters `Biomes`.
- JSON key names for the audio attribute objects beyond what `ambience.rs` and the bytecode accessors show. No code was built or run for this note (no cargo).
