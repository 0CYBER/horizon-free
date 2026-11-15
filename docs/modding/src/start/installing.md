# Installing and using mods

Mods are off until you install and turn them on. A fresh Horizon install has no mods folder and plays exactly
like vanilla 26.3.

## Installing a mod

Install a mod from the launcher's **Mods** page or from the game's **Options > Mods > Browse Mods**, or copy its
file by hand into the game folder (the folder that holds `options.txt`):

| Mod | Goes in | What happens next |
| --- | --- | --- |
| Content mod (`.hmod`, or a folder holding `horizon-mod.toml`) | `horizon-mods/` | Read when the game starts |
| WebAssembly mod (`.hmod` with `mod.wasm`) | `horizon-mods/` | Loaded into the game's sandbox when the game starts |
| Native mod (`.hmod` with Rust source) | `horizon-native-mods/` | The launcher builds it for that instance ([Native mods](../how-to/native-mods.md)) |

**Browse Mods** in the game lists content mods only, checked against the same registry as the launcher. Native and
WebAssembly mods install from the launcher or by hand.

Fabric and NeoForge jars do not run in Horizon: they target Minecraft's Java runtime, which Horizon does not have.
The data and assets inside a jar can be carried over; see
[Porting Java modpacks](https://github.com/horizon-rust/horizon/blob/main/docs/modpacks.md).

## Turning mods on

A newly installed mod is off.

- A mod installed from the launcher's **Mods** page turns on there, or with `horizon-launcher mods enable <project>`.
- A mod you copied in by hand turns on with a line `mod.<id>:true` in `horizon-mods.txt`, beside `options.txt`.

A content mod that holds only assets and reloadable data (textures, models, sounds, languages, recipes, tags,
loot, advancements and functions) starts working as soon as it is installed from Browse Mods: the game reloads
its resources, then the world's data as `/reload` does, LAN guests included. A mod that adds registry entries
or changes world generation, or that carries Java jar layers, waits for a restart. On someone else's server, all
content waits: the server owns its data.

Two content mods never own the same namespace. When two claim one, the mod whose file name sorts first loads, and
the log names the other and why it stayed out.

Benchmarks (`--benchmark`) never load mods.

## The Mods screen

The **Mods** button on the title screen (also **Options > Mods**) lists Minecraft and Horizon first, then each
installed mod with its icon, name and version. Search the list and sort it A–Z or Z–A. Select a mod to read its
id, version, license, authors, credits and description:

- **Homepage** and **Issues** open the mod's links, after asking you to confirm.
- **Config** opens the settings page of a mod that registers settings.
- A mod that is installed but off says so. A mod that could not load shows the reason in red.

The screen has no on/off switch; the launcher and `horizon-mods.txt` decide which mods run.

## Horizon's own mods

Horizon's optional extras are native mods in
[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods), under `native/<name>/`:

| Mod | Folder | Adds |
| --- | --- | --- |
| FPS Counter | `native/fps-overlay` | The frame rate on screen |
| Keystrokes | `native/keystrokes` | The movement and mouse keys you press |
| Zoom | `native/zoom` | A zoom key |
| Minimap | `native/minimap` | A minimap |
| Recipes | `native/recipes` | An item panel and recipe pages |
| Coordinates HUD | `native/coords-hud` | Position and facing, `/where` and `/coords`: a small example mod |

The game keeps the parts that must be fast, such as drawing the minimap's map, and the mod chooses the options.
