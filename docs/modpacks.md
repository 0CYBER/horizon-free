# Modpacks: porting any pack to Horizon

Goal (Ludovic, 2026-10-16): **support modpacks like All the Mods 10 far better than Minecraft
does, and be able to port any of them.** "Better" is measured: pack load time, RAM, FPS, TPS
and world-generation speed against the same pack on NeoForge or Fabric. "Any of them" means a
pipeline, not a list of hand ports. It takes a pack as players download it and produces native
Horizon mods. The pipeline is AI agents in dependency order, each port checked against the
original mod running in a dev-only Java instance.

This page is the design. It builds on [mods.md](mods.md) (the mod API, WASM in wasmtime) and
[research/java-mod-compat.md](research/java-mod-compat.md) (why Java jars and Mixins cannot
run in Horizon, so we port instead). The ATM10 inventory behind the numbers below is
[modpacks/atm10-mods.csv](modpacks/atm10-mods.csv).

## In one picture

```
 pack (.mrpack / CurseForge zip or instance / Prism instance)
   │  import: normalise to pack.json (MC version, loader, files, hashes, overrides)
   ▼
 inventory: every jar's metadata (mods.toml / fabric.mod.json, licence, deps, file census)
   │  classify: content · client-ui · library · performance · server-only · integration · other
   │  tier:     data · logic · machines · deep · client · none
   ▼
 dependency order (libraries first, then the mods that need them)
   │
   ├─ carry-over (mechanical, no agent): assets/ + data/ upgraded to 26.3 formats,
   │    block and item registrations from the oracle's facts export  →  <mod>.hmod
   ├─ port (one agent per mod, tiers logic and up): Rust → WASM against the Horizon
   │    mod API, iterating until its harness passes                  →  <mod>.hmod
   ├─ harness per mod: the Java mod in a dev-only NeoForge/Fabric oracle vs the port
   ▼
 coverage report: ported · partial · not yet · not needed, per mod and per class
```

Everything runs on the player's (or server owner's) machine, on jars they downloaded. The
results stay there. See [Licences](#licences).

## All the Mods 10, measured

**Source.** ATM10 is on CurseForge. Its GitHub repository (AllTheMods/ATM-10) carries the pack's
own mod list, `config/crash_assistant/modlist.json`, written by the Crash Assistant mod in the
pack. We used the version committed on 2026-10-15. It lists jar names, mod ids, versions,
CurseForge fingerprints and Modrinth SHA-1s. ATM10 8.x runs **Minecraft 1.21.1 on NeoForge
21.1.259**. The list has **495 jars** (494 mod ids: CC: Tweaked ships two jars) plus one
datapack (Universal Sawmill's).

**Metadata only.** Modrinth's `version_files` endpoint resolved 321 mods by hash, with their
categories, client/server sides, licence and required dependencies. A Modrinth name search
found 70 more CurseForge-only mods with a listing. The remaining 104 are CurseForge-only, so
CurseForge's API (which needs a key) would be the only source. They were classified from their
names and what they are known to do. 20 of those calls are marked `guess` in the CSV. No jar
was downloaded.

### Classes

| Class | What it is | Port? | ATM10 |
|---|---|---|---:|
| **content** | adds or changes gameplay: blocks, items, machines, mobs, worldgen, magic | yes: carry-over or agent | **287** |
| **library** | APIs and code shared by other mods (Architectury, Curios, GeckoLib, Kotlin for Forge, Balm...) | no: maps to the Horizon API or the SDK | **80** |
| **integration** | glue between two other mods (JEI plugins, AE2 ↔ Mekanism, Create add-on wiring) | after both sides are ported | **44** |
| **client-ui** | client-only screens, HUDs, tooltips, maps, recipe viewers | yes: client API, or built in | **32** |
| **other** | pack tooling and engine features: KubeJS, Almost Unified, Iris/shaders, CTM, crash/version tools | no: folded into the pipeline or the engine | **25** |
| **performance** | Sodium, FerriteCore, ModernFix, ImmediatelyFast, spark, AI and recipe speed-ups | no: Horizon is the replacement | **16** |
| **server-only** | admin and server features: ranks, essentials, backups, claims, spawn control | yes: server API | **11** |
| | | | **495** |

### Tiers (the API a port needs)

| Tier | Needs | ATM10 | of which content |
|---|---|---:|---:|
| **data** | registries (blocks, items) + carried-over data and assets; no code | 68 | 56 |
| **logic** | + behaviour callbacks (use, place, break, ticks), server events, config | 72 | 55 |
| **machines** | + block entities, menus and screens, capabilities (energy, items, fluids), mod networking | 101 | 89 |
| **deep** | + entities and AI, custom renderers and animated models, dimensions, new vanilla hooks (what Mixins do) | 91 | 87 |
| **client** | the client interfaces (`keys`, `target`, `inventory-read`, `ui`, `render`) | 38 | 0 |
| **none** | nothing to port | 125 | 0 |

Class by tier:

| | data | logic | machines | deep | client | none |
|---|---:|---:|---:|---:|---:|---:|
| content | 56 | 55 | 89 | 87 | | |
| library | | | | | | 80 |
| integration | 11 | 7 | 12 | 3 | 8 | 3 |
| client-ui | | | | | 30 | 2 |
| other | 1 | | | | | 24 |
| performance | | | | | | 16 |
| server-only | | 10 | | 1 | | |

What this says:

- **A quarter of ATM10 needs no port at all** (125: libraries, performance mods, pack tooling).
  Horizon's renderer and simulation replace the performance mods. Libraries become API
  features: Curios becomes accessory slots, GeckoLib an animated-model format, Patchouli,
  Modonomicon and GuideME a guide-book reader, the config libraries a config schema.
- **68 mods are data tier.** They are the structure packs (YUNG's, Moog's, Formations, CTOV,
  Towns and Towers, When Dungeons Arise), the decoration blocks (Macaw's doors, fences, paths,
  roofs, stairs; Chipped; XTones; Factory Blocks) and the ore and compression sets
  (AllTheOres, AllTheCompressed). They come over **without an agent** once registries and
  worldgen carry-over exist.
- **The big systems are deep tier.** Create, AE2, Mekanism, Ars Nouveau, Twilight Forest,
  MineColonies and Applied/Refined Storage are a long campaign of agent ports. They reach the
  most dependents: AE2 has 9 required dependents in the pack, Create 7, Ars Nouveau 6 and
  Mekanism 5.
- **Dependency depth is shallow.** Of the 321 mods with Modrinth data, 188 need nothing else
  in the pack, 103 sit one layer up, 29 two layers up and one (Apotheosis) three. The most
  required libraries are YUNG's API (10 dependents), GeckoLib, Resourceful Lib, SuperMartijn's
  core (8, 8, 7), then Curios, Bookshelf, Prickle and McJtyLib (6 each).

### Licences in ATM10

| Class | permissive (MIT, Apache, BSD, CC0, CC-BY) | copyleft (GPL, LGPL, MPL, AGPL) | All Rights Reserved | custom | non-commercial / no-derivatives | unknown (CurseForge only) |
|---|---:|---:|---:|---:|---:|---:|
| content | 93 | 46 | 62 | 16 | 9 | 61 |
| library | 27 | 20 | 9 | 6 | 4 | 14 |
| integration | 16 | 8 | 9 | 0 | 0 | 11 |
| client-ui | 13 | 5 | 3 | 5 | 1 | 5 |
| other | 6 | 8 | 4 | 1 | 0 | 6 |
| performance | 6 | 5 | 1 | 1 | 0 | 3 |
| server-only | 6 | 0 | 1 | 0 | 0 | 4 |
| **all** | **167** | **92** | **89** | **29** | **14** | **104** |

The "unknown" column is a gap in our metadata, not in the mods: NeoForge's `mods.toml` has a
mandatory `license` field, and the inventory reads it from each jar at import.

## Licences

Mods have their own licences: many are All Rights Reserved, others MIT, LGPL or MPL. A port is a
derivative work. These rules are part of the pipeline, not advice to users:

1. **Local by default.** The pipeline runs on the player's or server owner's machine, on jars
   they downloaded themselves. The ports it makes stay there. This works like `prepare` with
   Minecraft's own files: nothing is redistributed.
2. **The licence is recorded per mod** in the inventory (`license` from `mods.toml` or
   `fabric.mod.json`, plus Modrinth's SPDX id where one exists) and travels with the port into
   its `horizon-mod.toml` (`ported-from = { id, version, license, url }`).
3. **Publishing needs permission.** A port can go into a mod index, the launcher browser or a
   server's download list only if its licence allows redistributing derivatives (permissive,
   or copyleft with the port under the same licence and its source published) or the author
   has given written permission, recorded in the index entry. The tooling refuses to package
   anything else. No-derivatives and non-commercial licences (14 in ATM10) are never published.
   For them, even a local port gets a warning that the licence may not allow it.
4. **Mod code never enters this repository.** Mod jars, decompiled sources, Java oracle builds
   with mod code, and ports of mods we have no permission to publish all live outside the
   repo: in the user's pipeline work directory, or for our own development in
   `~/code/horizon-parity/modports/` (dev-only, never pushed). Ports are written from
   observed behaviour and the harness, with decompiled code as a local reference only. The
   repository holds the pipeline, the API, metadata (ids, names, versions, licences, counts)
   and test fixtures we write ourselves.
5. **Contacting authors or publishing any port is Ludovic's call.** Agents ask first.

This is engineering policy, not legal advice. Before anything is published, Ludovic decides,
with advice if needed.

## The mod API: what packs need, against API 0.1

API 0.1 is client-side only: `log`, `game` (player and clock), `chat`, `hud`, plus tick,
frame, chat and command hooks. Server mods do not run yet. What a content pack needs:

| Feature | API 0.1 | Needed by (ATM10 content+integration, by tier) | Planned in | Shape |
|---|---|---|---|---|
| Block registry: ids, states, properties, shapes, sounds, light | — | data and up (all 331) | **0.2** | declared in `data/<mod>/horizon/block/*.json`, each from a vanilla *template* block (below) |
| Item registry, block items | — | data and up | **0.2** | `data/<mod>/horizon/item/*.json`, from a vanilla template item |
| Recipes, loot tables, tags, advancements, lang, blockstates, models, textures, sounds in the mod's namespace | vanilla namespace only | data and up | **0.2** | the 26.3 data-pack and resource-pack formats, as they are |
| Worldgen JSON: features, structures, template pools, processors, structure NBT, biome additions | vanilla bundle only | about 30 data-tier structure and biome mods | 0.2.x | carried into the `worldgen/` bundle |
| Block and item behaviour callbacks: use, place, break, attack, random tick, scheduled tick, neighbour change, entity inside | — | logic and up (253) | **0.3** | `server` interface; hooks per registered id, budgeted |
| Server events, typed commands, game rules | — | logic and up, server-only (11) | **0.3** | `server` interface |
| Config: typed schema in the manifest, per world and per server, a settings page | — | most mods with code | **0.3** | `config` + the Options > Mods page |
| Block entities: per-block NBT state, ticking, client sync | — | machines and up (191) | **0.4** | `block-entity` interface, ticks batched per mod |
| Menus and screens: slots, progress bars, buttons, fluid and energy gauges | — | machines and up | **0.4** | a declared screen layout the game draws (`ui`), with server-side menu logic in the mod |
| Capabilities: energy (FE), item, fluid handlers on blocks and items | — | machines and up | **0.4** | `capabilities` interface; FE as the common unit, others as mod-defined resources |
| Data attachments on entities, chunks, levels | — | machines and up | **0.4** | `storage` per entity/chunk/level |
| Accessory slots (what Curios does) | — | Curios users (6 require it, more use it optionally) | **0.4** | built-in slots API |
| Mod networking between its client and server halves | — | machines and up | **0.4** | `net` interface (already planned in mods.md) |
| Custom recipe types: machine recipes with their own JSON | — | machines and up | **0.4** | recipe-type registry: the mod declares its JSON schema and a display layout for the recipe viewer |
| Entities: types, attributes, AI goals and brains, spawning | — | deep (87) | 0.5 | `entity` interface, AI in the mod with vanilla goals reusable |
| Custom renderers and animated models (block-entity, entity, item; GeckoLib's format) | — | deep | 0.5 | data models (Bedrock geometry + animations), plus a budgeted draw-list interface for the rest |
| Dimensions: dimension types, levels, chunk generators | — | about 10 (Twilight Forest, Aether, Ad Astra, Undergarden, ...) | 0.5 | JSON part with worldgen; custom generators as mod code |
| New hooks into vanilla behaviour (what Mixins do) | — | deep | as asked | each becomes a named hook, added for everyone ([java-mod-compat.md](research/java-mod-compat.md#3-a-porting-guide-outline)) |
| Client interfaces: `keys`, `target`, `inventory-read`, `ui`, `render`, `world-read` | planned | client tier (38) | 0.2–0.4 | as in [mods.md](mods.md#planned-interfaces) |

Two rules from mods.md stay: **the sandbox** (no files, sockets or processes; memory cap; fuel
per call; a crashing mod stops alone) and **free when unused** (no mods means vanilla code
paths and the vanilla resource root, so the parity harnesses test the same game).

### Registries 0.2: data-driven blocks and items

A `.hmod` may now carry `assets/` and `data/` beside (or instead of) `mod.wasm`. A content-only
mod needs no code:

```
ruby.hmod
├── horizon-mod.toml          id = "ruby", api = "0.2", side = "both", capabilities = ["registries"]
├── data/ruby/horizon/block/ruby_block.json
├── data/ruby/horizon/item/ruby.json
├── data/ruby/recipe/ruby_block.json            (26.3 data-pack formats)
├── data/ruby/loot_table/blocks/ruby_block.json
├── data/minecraft/tags/block/mineable/pickaxe.json   (adds to a vanilla tag)
├── assets/ruby/blockstates/ruby_block.json     (26.3 resource-pack formats)
├── assets/ruby/models/block/ruby_block.json
├── assets/ruby/items/ruby.json
├── assets/ruby/textures/block/ruby_block.png
└── assets/ruby/lang/en_us.json
```

A block declaration names a **template**, a vanilla block whose behaviour class, state
properties and shapes the new block takes, and overrides what differs:

```json
{
  "template": "minecraft:iron_block",
  "properties": { "hardness": 5.0, "explosion_resistance": 6.0, "sound": "minecraft:metal",
                  "map_color": 33, "light": 0, "requires_tool": true },
  "item": true
}
```

Most decoration and resource blocks are vanilla classes with other numbers: Macaw's doors are
`DoorBlock`s, XTones are full cubes, ore sets are `DropExperienceBlock`s. So templates cover the
data tier without code. Templates also give mod blocks every behaviour Horizon already
implements and the harnesses already check. Items work the same way (`"template":
"minecraft:diamond"`, overriding `max_stack_size`, `rarity` and so on).

How the game loads them, without touching vanilla's path:

- Horizon reads everything from one resource root. Resource packs already work as an
  **overlay root** (`packs::startup_root`) with vanilla's files linked and the overridden ones
  rewritten. Content mods add one more layer: the enabled mods' `assets/` and `data/`, and the
  prepared per-state tables (`facts.json`, `gameplay-facts.json`, `block-behaviours.json`, the
  collision, occlusion and movement tables, `model-states.json`, `render-definitions.json`,
  `items.json`, ...). Each table gets the mod blocks' rows appended, copied from the template's
  rows with the declared overrides. The loaders then read one tree and need no mod-specific
  code.
- **Vanilla state ids do not move.** Mod states are appended after vanilla's, mods in id order,
  so the same mod set gives the same ids. Saves store block names, so ids never reach the disk.
- Loaders that read only `data/minecraft/` (recipes, loot, tags) read every namespace, as
  vanilla does. With no mods there is only `minecraft`, so nothing changes.
- The overlay is cached like the resource-pack overlay and rebuilt when a mod file changes.
- **Limits.** Chunk sections store a block state in 16 bits: vanilla 26.3 has 35,723 states,
  so mods may add 29,813. Past that the content root is refused with the states per mod
  (Macaw's Doors, Fences, Paths and Trapdoors alone add 36,154: not all four fit). Block
  sprites fill texture arrays of 2048 layers, four at most (8,192; vanilla takes 1,250,
  identical textures share a layer). Wider state ids are the next step for big packs.
- Not in 0.2: multiplayer registry sync (single player and the integrated server first), worlds
  opened without a mod they used (vanilla turns unknown blocks into air; we will ask first),
  vanilla ids redefined by a mod (refused).

## The pipeline: `horizon-port-pack`

A Rust tool in this repo (`crates/horizon-port-pack`, a CLI the launcher also calls), modelled
on the version-port pipeline (`tools/version_port.py`: detect, diff, regen, harnesses, report,
publish). Each step reads and writes files in a work directory, so it can be rerun, resumed and
inspected:

```
<work>/                       ~/.horizon/ports/<pack-slug>/ for players; ~/code/horizon-parity/modports/ for us
├── pack.json                 normalised import
├── inventory.json            per mod: metadata, licence, census, class, tier, deps, layer
├── jars/                     the user's own jars (copied or linked from their instance), never uploaded
├── mods/<id>/
│   ├── carry/                assets/ + data/ upgraded to 26.3
│   ├── oracle/               facts, registries and traces exported from the Java oracle
│   ├── port/                 the agent's Rust crate (tiers logic and up)
│   ├── harness/              per-mod harness results and logs
│   └── <id>.hmod
└── coverage.json, coverage.md
```

**Status.** `import`, `inventory`, `fetch`, `carry` and `report` run today, and `port` runs
them all for what a player's machine can port (content mods of the data tier), as the
launcher's "Import modpack" does:

```
cargo run -p horizon-port-pack -- import ~/Downloads/pack.mrpack ~/.horizon/ports/pack
cargo run -p horizon-port-pack -- inventory ~/.horizon/ports/pack     # --offline reuses modrinth.json
cargo run -p horizon-port-pack -- fetch ~/.horizon/ports/pack         # the portable mods' jars, or name them
#   --from <folder>: copy the player's own jars (an installed instance, a downloads folder)
cargo run -p horizon-port-pack -- carry ~/.horizon/ports/pack         # content mods with a jar, or name them
#   --resources <prepared root>: only templates Horizon's tables have
cargo run -p horizon-port-pack -- report ~/.horizon/ports/pack        # coverage.json and coverage.md
cargo run -p horizon-port-pack -- port ~/Downloads/pack.zip ~/.horizon/ports/pack [--from <folder>] [--resources <root>]
```

`fetch` gets each jar the player's way, never from us: copied from a folder they have (matched
by SHA-1, or through a CurseForge app instance's `minecraftinstance.json`), else downloaded from
the link the pack itself gives (a `.mrpack`'s, on the format's hosts only) or, for a CurseForge
pack, the same file on Modrinth's public CDN, found by its SHA-1. A jar is kept only when its
hash matches. A file only CurseForge has is listed as needing a CurseForge API key (Ludovic's
decision; open question below), with the player's installed instance as the way round it.
`<work>/fetch.json` says what happened to each file. On All the Mods 10 8.3's CurseForge zip
(2026-10-17): of the 54 data-tier content mods, 42 come from Modrinth's CDN (84 MB) and 12 need
the key or the instance; `port` then carries the 42 over (0 ported, 42 partial: the harness is a
dev step).

`inventory` reads metadata only: Modrinth answers by file hash, and the jars only if the user's
instance has them (an imported instance folder, or jars put in `<work>/jars/`). Without jars,
classes come from the reviewed table, Modrinth categories and sides; with jars, from their
`mods.toml` / `fabric.mod.json` and file census too. On Create+ 6.0.0 alpha f (155 files, metadata
only): content 54, library 33, client-ui 26, performance 25, other 14, unknown 2, server-only 1;
40 rows matched the reviewed ATM10 table; three dependency layers (93 / 57 / 5).

`carry` copies a jar's `assets/<id>/`, `data/<id>/`, `data/minecraft/tags/` and `data/c/tags/`
into `mods/<id>/carry/`, a folder content mod (API 0.2) the game loads with `--dev-mod`, and
keeps what a content mod may not hold (other namespaces, vanilla data) in `outside/`. Files of
the mod's own content that the game's content loader would refuse whole also go to `outside/`,
listed with the reason in the report's `unreadable`: paths vanilla does not read (upper case),
JSON that does not parse, and shared tags with `"replace": true`. Vanilla skips them one by one.
A byte-order mark is dropped. For ATM10's 42 mods that is 19 files in 7 mods, and the 42 load as
carried. No class file is copied. It upgrades from 1.21.1 to 26.3: vanilla-type recipes (ingredients), loot tables
(`conditions` lists become one `condition`, `functions` become `modifier`, kinds move to `type`,
tool matches on silk touch or shears become `minecraft:tool/can_silk_touch` and
`minecraft:tool/can_shear`), and the `items/` definitions 1.21.4 introduced.

Blocks and items come from the NeoForge oracle when `<work>/oracle/facts.json` exists (step 6):
every block the mod registers gets the vanilla template of the same vanilla class and states
whose shapes, sounds, set type and wood type match best, with the mod's own hardness, blast
resistance, light, tool need, friction, map colour and sounds as overrides. `--resources
<prepared>` limits templates to blocks Horizon's tables have. `carry-report.json` lists, per
block, what the template does not reproduce (other shapes, sounds no vanilla block has, a class
of its own), and the blocks left out: those whose vanilla class has no vanilla block with the
same states (Macaw's garage doors, multi-part gates). Recipes get the convention tags they use
(`c:dyes/red`), filled from the oracle with vanilla's and the mod's own members. Without the
oracle, templates are guessed from the blockstate's properties: the template whose properties
include them with the fewest left over (models often ignore `powered` or `waterlogged`), the
block's name breaking ties (`_pane`, `_gate`, `_trapdoor`).

### 1. Import

| Format | Read | Jars come from |
|---|---|---|
| Modrinth `.mrpack` | `modrinth.index.json`: `dependencies` (minecraft, neoforge / fabric-loader / forge / quilt-loader), `files[]` with path, sha1/sha512, `env` (client/server), download URLs; `overrides/`, `client-overrides/`, `server-overrides/` | downloaded from the listed Modrinth URLs, hash-checked |
| CurseForge zip | `manifest.json`: `minecraft.version`, `modLoaders[]`, `files[]` (`projectID`, `fileID`, `required`), `overrides`. When the overrides hold Crash Assistant's `config/crash_assistant/modlist.json` (All the Mods 10 does), its jar names, mod ids and SHA-1s replace the bare ids | the user's installed instance (CurseForge app `minecraftinstance.json` + `mods/`), or the same file on Modrinth by SHA-1. CurseForge's API needs a key, so we take what the user already downloaded |
| Prism / MultiMC instance | `mmc-pack.json` components (`net.minecraft`, `net.neoforged`, `net.fabricmc.fabric-loader`), `instance.cfg`, `.minecraft/mods/*.jar`, packwiz `.pw.toml` files if present | the instance folder |
| packwiz | `pack.toml`, `index.toml`, `mods/*.pw.toml` | as listed |
| Crash Assistant `modlist.json` | jar name → mod id, name, version, Modrinth SHA-1; the loader entry (NeoForge `21.1.x` is Minecraft 1.21.1). Packs such as ATM10 publish it in their repository | metadata only; jars put in `<work>/jars/` |

`pack.json` holds the Minecraft version, the loader and its version, each file with its hashes
and side, and the overrides (configs, KubeJS scripts, datapacks).

### 2. Inventory and classification

Per jar, without decompiling:

- **Metadata:** `META-INF/neoforge.mods.toml` / `META-INF/mods.toml` / `fabric.mod.json`: id,
  version, `license`, authors, URLs, dependencies with ordering and side, mixin configs, access
  transformers.
- **Census of files:** blockstates (≈ blocks), item models (≈ items), recipes, loot tables,
  tags, worldgen files, structure NBTs, languages, sounds, textures; the number of mixins and
  the vanilla classes they target; whether the jar has code beyond its data.
- **Census of classes:** the superclass names in the class files (constant pool only): how
  many block entities, menus, screens, entities, renderers, recipe serializers, payload types.
- **Known mods:** a reviewed table in the repo (id → class, tier, Horizon equivalent: "sodium:
  performance, built in", "jei: client-ui, recipe viewer built in") wins over heuristics. It
  starts with ATM10's 495 rows.

Class rules, in order: the known-mods table; client-only side → client-ui; server-only side →
server-only; mixins only, into rendering or ticking, no content → performance; no content and
depended on by others → library; depends on two or more content mods and adds little →
integration; otherwise content. Tier rules: content with no code beyond registration →
data; behaviour callbacks only → logic; block entities or menus → machines; entities,
renderers, dimensions or mixins into gameplay → deep.

### 3. Dependency order

A topological order over required dependencies, libraries first. Each mod gets a layer (0:
needs nothing in the pack). A mod is ported only after its dependencies are ported or mapped
to the API. A library mapped to the API ("Curios → accessory slots") counts as done when that
API feature exists. Cycles (rare: optional mutual integrations) are broken at optional edges.

### 4. Carry-over: data and assets

Mechanical, the same for every mod, and no agent:

1. Extract `assets/` and `data/` from the jar into `mods/<id>/carry/`.
2. **Upgrade to 26.3.** ATM10 is 1.21.1 and Horizon is 26.3. Item models moved to `items/`
   (1.21.4), recipe ingredients and results changed shape (1.21.2), some definitions moved to
   files of their own, and some registries and tags were renamed. The upgrader is a table of these changes generated from
   vanilla's own datafixers and pack-format notes, applied per file. A file it cannot upgrade
   is listed, not guessed.
3. **Registrations.** Block and item ids registered in Java code are read from the oracle
   (step 6): its facts exporter writes each mod block state's hardness, sounds, shapes, light,
   map colour and class chain. These are the same exporters that produce Horizon's vanilla
   `gameplay-facts.json`, run on the modded game. The class chain picks the template (a
   `DoorBlock` → a vanilla door). Without an oracle, the blockstate's property set gives the
   template (`facing, half, hinge, open, powered` is a door), with the template's numbers.
4. **The pack's own data.** KubeJS scripts, Almost Unified and config-driven recipes change
   recipes, tags and loot after load. We do not translate scripts. The oracle exports the
   **final** recipe, tag and loot registries of the whole pack once, and the pack layer
   carries those over.
5. Write `<id>.hmod`. For a data-tier mod, it is now ported.

### 5. Code: one agent per mod

For tiers logic, machines and deep, in dependency order, as many agents at once as the AI
budget allows:

- **Input:** the inventory record, the carried-over data, the oracle's exports and traces, the
  harness, the Horizon mod API docs, the ports of its dependencies, and the mod's decompiled
  code as a local reference (dev-only, in the work directory).
- **Output:** a Rust crate in `mods/<id>/port/` built with `cargo horizon-mod build`, and its
  `.hmod`.
- **Loop:** write → build → run the harness → read the differences → fix, until the harness
  passes or the agent reports what blocks it. Usually that is an API gap, which becomes an
  issue against the mod API ("needs a scheduled-tick hook"), the way version ports turn
  differences into work items.
- **Rules:** the agent writes the port from observed behaviour and the harness, not by
  transliterating decompiled Java; ports follow the mod's licence ([Licences](#licences)).
  Players without an AI agent get the carry-over tiers. A port the licence lets us publish can
  be downloaded instead of regenerated.

### 6. The Java oracle and per-mod harnesses

The oracle is **dev-only**, like Horizon's vanilla oracles in `tools/`: the pack's own
Minecraft version (1.21.1 for ATM10) with NeoForge or Fabric, the mod and its dependencies,
plus a small exporter mod (written by us, never shipped to players). It runs headless and is
driven over a socket, as the vanilla oracles are.

A **harness per mod**, generated from the tier:

| Tier | What is compared, oracle vs Horizon with the port |
|---|---|
| data | every registered block state's facts (hardness, sounds, shapes, light, map colour, drops via loot), every item's facts, every recipe's result for its inputs, loot table outputs over fixed seeds, tags; structure placement over fixed seeds and chunks |
| logic | scripted interactions: place, use, break, tick N times; the world diff and the inventories after each step |
| machines | a fixture world with the machine, its inputs and power: the output, energy and progress per tick over N ticks; the menu's slots and data values; NBT after save and load |
| deep | entity scenarios (spawn, tick N, positions and states, drops), and render captures where the look is the point, compared as images with a tolerance |

The comparison skips vanilla 1.21.1-versus-26.3 differences: it normalises ids and components
through the step-4 upgrader and compares only the mod's own ids and effects.

**Status: the data-tier harness runs.**

```
python3 tools/neoforge_oracle.py export --jars <mods> --deps <their libraries> --out <work>/oracle/facts.json
cargo run -p horizon-port-pack -- carry <work> --resources .local/horizon-client/26.3
python3 tools/mod_port_harness.py --work <work>      # or --jars/--deps to export first
```

- `tools/neoforge_oracle.py` installs a NeoForge server (21.1.259, ATM10's) under
  `.local/neoforge-oracle/`, builds the exporter mod in `mods/horizon/src/neoforge-oracle/` (our code, dev
  only), starts the server headless with the jars and writes, for every block of the mods'
  namespaces and every vanilla block: the class chain, states, per-state hardness, light, tool
  need, map colour, sounds, outline and collision shapes; drops for hand, diamond tools, shears
  and silk touch; placement on a floor and a wall; BlockSetType and WoodType. Then items (class,
  stack size, rarity, food), recipes (type, result, count, resolved ingredients) and tags. One run
  with the seven mods below takes about 30 s.
- `crates/horizon-client/examples/mod_port_probe.rs` builds the content root from the carried
  mods and dumps what Horizon's loaders make of the same blocks, drops, recipes and tags.
- `tools/mod_port_harness.py` compares them per mod (blocks, properties, sounds, shapes, drops,
  placement against the template's in the oracle, vanilla tags, recipes, items) and writes
  `<work>/mods/<id>/harness/result.json`. It maps 1.21.1 sound events to 26.3's through the
  vanilla blocks both versions have (`block.metal.*` became `block.iron.*`) and ignores vanilla
  items 1.21.1 did not have (pale oak in `#minecraft:logs`).

The oracle needs the mods' jars, so it is not a `tools/parity.py` harness: it runs where the
user's jars are, and its facts and results stay in the work folder. The first real mods
(ATM10's own jars from Modrinth, all MIT; checks passed / compared):

| mod | blocks | properties | sounds | shapes | drops | placement | tags | recipes | items |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Macaw's Trapdoors 1.1.5 | 179/179 | 11456/11456 | 11456/11456 | 11456/11456 | 10024/10024 | 358/358 | 534/534 | 191/191 | 191/191 |
| Macaw's Doors 1.1.5 | 239/245 | 15296/15296 | 15296/15296 | 12352/15296 | 13384/13384 | 478/478 | 941/941 | 257/257 | 251/257 |
| Macaw's Fences and Walls 1.2.1 | 150/180 | 4800/4800 | 4800/4800 | 4032/4800 | 33600/33600 | 300/300 | 432/432 | 222/222 | 150/180 |
| Macaw's Paths and Pavings 1.1.1 | 234/315 | 4602/4602 | 4602/4602 | 4472/4602 | 6734/6734 | 468/468 | 416/416 | 315/315 | 234/315 |
| Glassential-renewed 3.4.7 | 105/114 | 5177/5177 | 5177/5177 | 4727/5177 | 6447/6447 | 210/210 | 105/105 | 143/143 | 106/115 |
| Factory Blocks 1.4.0 | 42/49 | 42/42 | 0/42 | 42/42 | 294/294 | 84/84 | 42/42 | 1/1 | 42/49 |
| Cloud Glass 1.1.0 | 16/16 | 16/16 | 16/16 | 0/16 | 112/112 | 32/32 | – | 31/31 | 16/16 |

What fails is what API 0.2 cannot express yet: shapes of their own (Japanese doors, hedges,
paths a sixteenth high, glass without collision), sound events of the mod's own
(`factory_blocks:metal_sound`), and blocks whose vanilla class has no vanilla block with the
same states (garage doors, double gates, engraved and facing paths, fans). Every drop, recipe
and tag of the declared blocks matches.

### 7. Coverage report

`coverage.json` and `coverage.md`, per mod and totalled per class and tier:

- **ported**: `.hmod` built, harness green;
- **partial**: `.hmod` built, harness passes the listed parts (e.g. blocks and recipes yes,
  machine processing not yet), with the failing checks and the API gaps blocking them;
- **not yet**: waiting on an API tier or a dependency, which is named;
- **not needed**: performance, library mapped to the API, pack tooling folded into the
  pipeline, client-ui built in.

Plus the licence summary (how many ports could be published) and, for a pack, "playable?":
whether every required content mod is ported or partial with its blocks and items present.

`horizon-port-pack report <work>` writes the per-mod rows and the class, tier and licence totals from the inventory, each mod's `carry-report.json`
and its harness result. A data-tier mod is **ported** when its harness is green and nothing was
left out (blocks with a class of their own are counted and named, since facts cannot show
their extra code); **partial** with the failing checks, the blocks left out and what the
templates miss; **not yet** with the API tier it waits for, or the dependency; **not needed**
for libraries, performance mods, pack tooling and what Horizon has built in. Unclassified mods
(the 20 CurseForge-only rows the reviewed table only guesses) count as not yet.

**ATM10, measured** (`import` of ATM10's `modlist.json` from its repository on 2026-10-16, 496
files; `inventory`; `carry` and the harness on the eight mods we downloaded; `report`):

| class | ported | partial | not yet | not needed | all |
|---|---:|---:|---:|---:|---:|
| content | 1 | 7 | 268 | 0 | 276 |
| library | 0 | 0 | 0 | 77 | 77 |
| integration | 0 | 0 | 40 | 3 | 43 |
| client-ui | 0 | 0 | 27 | 2 | 29 |
| other | 0 | 0 | 1 | 23 | 24 |
| unknown | 0 | 0 | 20 | 0 | 20 |
| performance | 0 | 0 | 0 | 16 | 16 |
| server-only | 0 | 0 | 11 | 0 | 11 |
| **all** | **1** | **7** | **367** | **121** | **496** |

By tier, the 367 not yet: data 57 (carry-over and the harness, no agent: their jars were not
downloaded), logic 66 (API 0.3), machines 97 (0.4), deep 92 (0.5), client 35, unclassified 20.
Ported: Macaw's Trapdoors. Partial: Macaw's Doors, Fences and Walls, Paths and Pavings,
Glassential, Factory Blocks, Cloud Glass (see the harness table) and MVS, whose 784 worldgen
files are carried but not yet read by the game. Of the 8, 7 are MIT and one LGPL; nothing is
published.

## Measuring "10x"

`tools/perf_bench.py` already measures Horizon against vanilla and vanilla with performance
mods on the same machine, world and camera path ([benchmark.md](benchmark.md)). With
`--game neoforge --mod-jars <dir> --deps <dir>` it runs the mods on their own loader (NeoForge
21.1.259 for ATM10, installed under `.local/neoforge-oracle`, `-Xmx2G` like the launcher),
driven by the same steps through a dev-only NeoForge port of the bench mod
(`mods/horizon/src/neoforge-bench`). `--game horizon --port-work <work>` loads a work folder's
carried-over mods into Horizon. A whole pack (`--pack`, its recommended JVM arguments) is next.

| Metric | How | Horizon side |
|---|---|---|
| **Load time** | start → title screen; start → in a fresh world, every chunk around the spawn drawn | the same with the ported pack's `.hmod`s enabled |
| **RAM** | peak and steady RSS of the process, sampled 4 times a second from outside (the whole JVM for Java) | the whole process |
| **FPS** | the four flight scenes: mean, 1 % and 0.1 % lows, p99 frame time | same |
| **TPS / MSPT** | a fixed server-load world: N loaded chunks with the pack's machines and mobs placed by the harness fixtures; mean and p99 milliseconds per tick over 60 s (NeoForge: `/neoforge tps` and spark; Horizon: the integrated server's tick times, which today feed only the debug chart, exposed through `horizon-ctl`) | same fixture, the ported content |
| **World generation** | chunks per second pregenerating a fixed square (Chunky on NeoForge; Horizon gets a pregeneration command for this) with the pack's worldgen | same |
| **Disk and download** | install size of the pack against its `.hmod`s | |

Two comparisons, both published:

1. **Same content.** NeoForge with only the mods Horizon has ported (plus their dependencies)
   against Horizon with those ports. Content matches, so this is the honest ratio.
2. **What players run today.** The full pack on NeoForge as a reference row, labelled as
   such, beside Horizon's partial pack. This is not a like-for-like ratio.

"10x" is claimed per metric only where the same-content ratio reaches it over the repeats'
confidence interval. Other ratios are reported as they come out. Caveat (in every report):
the pack runs Minecraft 1.21.1 and Horizon runs 26.3, so the vanilla parts differ by a few
versions.

### Measured: six ported mods against the same mods on NeoForge

Cloud Glass, Factory Blocks, Glassential, Macaw's Fences, Paths and Trapdoors (Doors left out:
with Trapdoors it passes the block states chunks hold, see Limits), carried over and loaded as
dev mods in Horizon, against their jars plus Architectury and Fusion on NeoForge 21.1.259.
Mac mini M4 Pro, 1280x720, seed 1234, the four flight scenes, 2 games per row, 2026-10-17.
Means; FPS is the mean of the four scenes. "Title" is start to title screen, "world" start to
a fresh world drawn around the spawn, "scene load" a teleport to every chunk drawn.

| RD | game | title s | world s | scene load s | FPS | RSS steady / peak MB | load avg |
|---|---|---:|---:|---:|---:|---:|---:|
| 8 | Horizon + 6 mods | 9.8 | 16.2 | 4.0 | 122 | 2,644 / 2,771 | 32 |
| 8 | NeoForge + 6 mods | 19.5 | 50.0 | 30.2 | 72 | 2,325 / 2,592 | 56 |
| 8 | Horizon | 5.0 | 11.0 | 4.0 | 124 | 1,559 / 1,629 | 51 |
| 8 | NeoForge | 12.8 | 36.1 | 21.9 | 104 | 2,334 / 2,558 | 30 |
| 16 | Horizon + 6 mods | 10.8 | 27.0 | 11.3 | 90 | 2,925 / 3,032 | 40 |
| 16 | NeoForge + 6 mods | 26.4 | 82.5 | 64.3 | 25 | 3,016 / 3,519 | 45 |
| 16 | Horizon | 3.5 | 10.0 | 7.4 | 119 | 1,732 / 1,871 | 56 |
| 16 | NeoForge | 16.9 | 73.7 | 43.2 | 36 | 2,748 / 3,425 | 48 |

Same content, Horizon against NeoForge: the world is ready 3.1x sooner at 8 chunks and 3.1x at
16, the title screen 2.0x and 2.4x sooner, and the flights run 1.7x and 3.6x the frames. RAM is
not lower: Horizon's steady RSS grows 1.1 GB with the mods (61,816 block states against
vanilla's 35,723; the per-state tables are the next thing to measure), while the JVM stays
inside its 2 GB heap. Caveats, so none of this is a "10x" claim yet: other sessions and CI
runners kept the machine's load average between 30 and 56 on 14 threads, so the confidence
intervals are as wide as the means (`perf_bench.py --table`); Horizon's frames are paced near
the display's 144 Hz at 8 chunks; Horizon's title time includes building the content overlay on
a first start (about 5 s, cached afterwards); and the pack runs 1.21.1 against Horizon's 26.3.
A quiet rerun with more games is the next measurement.

## The working version for 2026-10-23

In scope (each is a PR or a small stack):

1. **This design merged.**
2. **Registries 0.2** in the game: content-only `.hmod`s with blocks (full cubes and the
   vanilla templates the first pack needs: doors, trapdoors, fences, walls, slabs, stairs,
   panes, glass), items, block items, recipes, loot tables, tags, blockstates, models, textures
   and lang, in single player. Proven by a harness: a test mod's block placed, rendered, broken
   with the right time and tool, dropping its loot, crafted from its recipe; every per-state
   fact equal to its template's with the overrides applied. Parity unchanged with no mods.
3. **`horizon-port-pack`**: import (`.mrpack`, CurseForge zip and instance, Prism), inventory,
   classify, licence record, dependency order, data carry-over for data-tier mods and their
   `.hmod`s, coverage report. Run on ATM10's real list (coverage report published in the PR,
   metadata only) and on a small real pack end to end.
4. **First pack end to end, locally:** 5–10 data-tier mods with permissive licences from ATM10
   (candidates: Macaw's Doors, Fences and Walls, Trapdoors and Paths (MIT), Factory Blocks
   (CC0), XTones Reworked (MIT), Cloud Glass (MIT), Glassential (MIT), Crystalix (LGPL)), each
   with its harness against the NeoForge 1.21.1 oracle. The ports stay local even though these
   licences would allow publishing: we ask first.
5. **Launcher:** "Import modpack" (`.mrpack` / CurseForge zip / instance) showing the
   coverage report (ATM10's first), and "Port and play" for what is portable.
6. **Measured:** load time, RAM and FPS of that pack on NeoForge 1.21.1 against Horizon, with
   the full ATM10 on NeoForge as the reference row.

Out of scope for that date, and next in order: worldgen carry-over (the 30 or so structure and biome mods;
the first one looked at, Moog's Voyager Structures, declares all 130 structures and 116 structure
sets with MoogsStructureLib's own `generic_jigsaw_structure` and `advanced_random_spread` types and
NeoForge's `c:` biome tags, so its carry needs a mapping to vanilla's jigsaw and random spread
types plus the oracle's biome tags, not only bundle merging),
behaviour callbacks and server events (0.3), block entities, menus, capabilities and net
(0.4), the first agent-ported logic-tier mod as the pilot of step 5, then machines. Porting all
of ATM10 is a campaign. Each step publishes its coverage number; we make no claim beyond what
the report shows.

## Open questions for Ludovic

- Contacting authors of permissively licensed mods to publish ports (none contacted).
- Whether players should run code-port agents themselves (their own Claude Code or Codex
  login, as Zenith does) or only download published ports.
- A CurseForge API key for the launcher (CurseForge grants them to launchers on request). Until
  then, CurseForge packs import from an installed instance.
