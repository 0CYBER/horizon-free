# Mods and data packs

Horizon has its own mod system, written for Horizon and not compatible with Java mods. Data packs work in part. This page says what you can do with each today.

## Mods

Horizon mods are written in Rust, compiled to WebAssembly and run in a sandbox. A mod can only do what it asks for and you allow, and a mod that crashes is switched off without taking the game down.

> **Early.** The mod system is new. There is no store for Horizon mods yet: they are files to put in a folder. The launcher's **Mods** page (see below) finds resource packs and modpacks, not `.hmod` mods.

**Playing with mods**

1. Put the mod's `.hmod` file in the `horizon-mods` folder of the game folder (see [Install](/docs/install#first-launch) for where that is), or use the **Open Mod Folder** button on the Mods screen.
2. Open **Options, Mods...**. Each mod is listed with what it may do ("May see your position, health and the time, draw text on your screen…") and a switch. Mods start off. Turn on the ones you trust.
3. A mod that failed to load shows why, in red, on its row.

**What a mod can do today**

- **Client mods** read the same things the F3 screen shows (your position, health, food, the time), draw text panels on the HUD, show chat lines only you see, and add chat commands. They work in single player and on any server, and a server cannot see them.
- **Content mods** add blocks and items from data and assets: models, textures, recipes, loot tables and tags, with no code. They work in single player today. This is also what the launcher's [modpack](/docs/modpacks) carry-over produces.
- **Server mods**, which run where the world is simulated, are planned.

**Making a mod**

```sh
rustup target add wasm32-unknown-unknown
cargo install --git https://github.com/ludovic111/horizon cargo-horizon-mod
cargo horizon-mod new my-mod
cd my-mod
cargo horizon-mod run
```

`run` builds the mod and starts the game with it. After an edit, `cargo horizon-mod build` rebuilds it and the running game reloads it. The [Mod API](/docs/api/mods) lists what a mod can do today, its [reference](/docs/api/mods-reference), the [ways in](/docs/api/mods#ways-in) (Rust today, Lua scripts planned) and what comes next; the full guide, with the manifest and the runtime, is [docs/mods.md](https://github.com/ludovic111/horizon/blob/main/docs/mods.md). Java mod jars don't run in Horizon: their code calls and patches Minecraft's Java classes, and Horizon has none. What carries over is the data and assets inside a jar ([modpacks](https://github.com/ludovic111/horizon/blob/main/docs/modpacks.md)).

## Finding resource packs and modpacks

The launcher's **Mods** page (launcher 0.17 and newer) searches [Modrinth](https://modrinth.com) for what Horizon can load: resource packs and modpacks. Each result says how well it runs in Horizon (**Works**, **Partly**, or **Doesn't run**), and by default the page shows only what runs. **Install** downloads the file, checks it against the checksum Modrinth lists and refuses anything that does not match, then puts it in the right folder, with the packs it needs. **Update** and **Remove** are there too. A resource pack still has to be turned on in the game's Resource Packs screen. A modpack is imported on the [Modpacks page](/docs/modpacks).

CurseForge support is coming soon: the page shows it, but doesn't search it yet. The same things work from a terminal: `horizon-launcher mods search`, `install`, `update` and `remove`.

## Java mods from the Mods page (next launcher release)

Java code does not run in Horizon, but much of what a mod adds is data: blocks, items, recipes, loot tables, tags, worldgen, textures. From the next launcher release, the Mods page also lists Fabric, NeoForge and Forge mods made for 26.3 and, before offering one, downloads its jar, checks it against Modrinth's checksum and reads it (without running it) to say what of it Horizon runs:

- **Works**: the mod is only content, and all of it carries over.
- **Partly**: its content carries over; the row lists what does not, such as machines and containers, mobs, blocks or items with behaviour of their own, recipe types of its own, or Mixins.
- **Doesn't run**: there is nothing to carry over (performance mods, libraries, mods that are all code). It is not offered.

**Install** brings the mod over into `horizon-mods` and turns it on; start the game again to load it. Blocks take the closest vanilla block's shape, hardness and sounds, so a carried-over block can break faster or slower than in Java. Most popular mods today are **Partly** or **Doesn't run**: the verdict is the honest answer for each one.

## Data packs

A world's data packs go in its `datapacks` folder, as folders or `.zip` files with a `pack.mcmeta`, as in vanilla.

**Works today (single player and open-to-LAN worlds):**

- **Functions** (`.mcfunction`, with macros and line continuation), **function tags** (such as `minecraft:load` and `minecraft:tick`), **predicates** and **item modifiers**.
- The `/function`, `/datapack` and `/reload` commands, and the choice of which packs are on, kept with the world.

**Not applied from data packs yet:** custom recipes, loot tables, advancements and world generation. Horizon reads those from the vanilla data it prepared, and from content mods.

Vanilla's own data is always on, and it matches vanilla: every command runs as vanilla does, and recipes, loot and world generation are checked against the real game.
