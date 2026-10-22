# The launcher's mod browser

Search Modrinth (CurseForge support is coming soon) for Java mods, modpacks, resource packs and shader
packs, see how well each one runs before installing it, and install it with its checksum checked. This is item 16 of the roadmap and issue #577. The code is in
`launcher/src/browser.rs` and `launcher/src/browser/`.

## What it offers

Horizon doesn't run Java code ([research/java-mod-compat.md](research/java-mod-compat.md)). It ports what a Java mod
carries as data (blocks, items, recipes, loot tables, tags, worldgen, textures, names) with `horizon-port-pack`, and
says before install which of the mod's features that leaves out. Horizon `.hmod` mods aren't on Modrinth or CurseForge
yet (Modrinth has no `horizon` loader).

| Kind | Goes to | How well it runs |
|---|---|---|
| Java mods (Fabric, NeoForge, Forge, Quilt) | ported into `horizon-mods/<mod id>/` and turned on | Checked from its jar before install: **Works**, **Partly** or **Doesn't run** ([below](#java-mods)) |
| Resource packs | `resourcepacks/` beside `options.txt` | **Works** when made for 26.3 ([resource-packs.md](resource-packs.md) lists what Horizon reads). Older packs are **Partly**: they load with vanilla's compatibility warning |
| Shader packs (Iris, OptiFine) | `shaderpacks/` | **Partly**: translated to WGSL, the full-screen passes draw and the world and shadow passes don't yet ([shader-packs.md](shader-packs.md)). Packs for other loaders (Canvas, core-shader packs) **don't run** and can't be installed |
| Modpacks (`.mrpack`, CurseForge zip) | the Modpacks page's import | **Partly**: Horizon runs the mods it can port, from the player's own downloads ([modpacks.md](modpacks.md)) |

By default a search shows only what runs: Java mods made for 26.3, resource packs made for 26.3, shader packs for Iris
or OptiFine, and every modpack. **Show everything** lists the rest of the kind too, each marked with how well it runs; what doesn't run is
never offered for install.

## Java mods

Before a mod is offered for install, the launcher reads its jar. For each result it picks the version it would
install (the newest for 26.3; with **Show everything**, else the newest for 1.21 or later, the versions port-pack
upgrades from), downloads that file with the checks below, and runs `horizon-port-pack check` on it
([modpacks.md](modpacks.md#the-pipeline-horizon-port-pack)): the jar is carried over into a folder content mod and its
class files are read, not run, for what the carry can't take. Two checks run at once; the row says **Checking** until
its answer comes.

- **Works**: content only. Its blocks, items, recipes, loot, tags and worldgen port, and its code only registers
  them. Blocks take the closest vanilla block's shape, hardness and sounds (a block's numbers live in Java code; the
  NeoForge oracle that reads them for modpacks is a developer tool).
- **Partly**: the content ports; the row lists what doesn't: blocks with no vanilla block to copy, recipes of the
  mod's own types, block entities (machines, containers), menus, mobs and entities, blocks and items with behaviour
  of their own, worldgen written in Java, Mixins, the game events it reacts to.
- **Doesn't run**: nothing to port (performance mods, libraries, everything in code), a jar made for 1.20 or older,
  or a mod id with characters Horizon mods can't use (`fabric-api`). Never offered for install.

The answer is kept in `mod-browser/checks-1/<checksum>/` under the game folder, with the carry when something
ports; the jar itself isn't kept. A file is checked once. On ten 26.3 jars a check took 0.2 to 3 s (24 s for an
18 MB jar with 9,360 recipes).

**Install** copies the carry into `horizon-mods/<mod id>/`, turns it on in `horizon-mods.txt` (the game's Options >
Mods lists it) and records it in `mod-browser.json` with its verdict and what didn't port, which the Installed list
shows. Its required mods that port are installed with it; one with nothing to port (a library such as Cloth Config
or GeckoLib) is listed as not ported. A mod that shares its id or a namespace with an installed one is refused. The
game reads mod content when it starts: start it again after installing. **Remove** deletes the folder and turns it
off.

## Checks on every download

- The file must come from Modrinth's or CurseForge's file servers (`cdn.modrinth.com`, `edge.forgecdn.net`,
  `mediafilez.forgecdn.net`), over HTTPS.
- Its name must be one plain file name (no folders, `..`, hidden names or reserved characters).
- It is written as `.<name>.part`, hashed as it arrives, and renamed into place only when its size and checksum match
  what the source lists: SHA-512 on Modrinth, SHA-1 on CurseForge (the strongest hash CurseForge publishes). Anything
  else removes the part file and installs nothing.
- A file whose source lists no checksum is refused.

## The Mods page

The launcher's **Mods** page has the four kinds as tabs (Mods, Modpacks, Resource packs, Shader packs), a search field (Enter searches), **Show everything** and
the source: Modrinth, with **CurseForge (coming soon)** beside it, disabled. Each result is an index row: its summary, author,
downloads, source and licence, how well it runs (and why), **Page** (the project's page on its source) and
**Install** (**Import** for a modpack). The Modpacks tab lists the imported packs first, with their coverage
report, **Port** and **Play** ([modpacks.md](modpacks.md)). Installed packs are listed above the results with their version, source,
file and the checksum they were checked against, with **Update to …** when a newer version runs and **Remove**
(asked twice).

## Installing, updating, removing

`mod-browser.json` beside `options.txt` lists every file the browser installed: its source, project, version, file
and the checksum it was checked against. The browser replaces or removes only the files it lists; a file of the
same name the player put there is never overwritten.

- **Install** picks the newest version that runs, then its required dependencies, checked before any download: a
  dependency that isn't a pack Horizon loads (a Java mod) stops the install. Dependencies go in first. A resource
  pack still has to be turned on in the game's Resource Packs screen, and a shader pack in Video Settings > Shader
  Packs, as in vanilla and Iris.
- **Update** installs the newer version and removes the old file.
- **Remove** deletes the file and its entry. Packs installed as dependencies stay.
- **A modpack** is downloaded (checked the same way) into `ports/downloads/` and imported; the Modpacks tab says
  what of it Horizon runs and ports it.

From a terminal (or an agent):

```sh
horizon-launcher mods search faithful                 # --kind mod|modpack|resource|shader, --everything, --source
horizon-launcher mods check macaws-fences-and-walls   # a Java mod's verdict, what ports and what doesn't (--json)
horizon-launcher mods install faithful-32x            # id or slug; a Java mod is ported, a modpack imported
horizon-launcher mods list                            # --json
horizon-launcher mods update [faithful-32x] [--check]
horizon-launcher mods remove faithful-32x
```

## Sources

**Modrinth.** The public API (`api.modrinth.com/v2`) needs no key. The launcher names itself in its user agent, as
Modrinth asks.

### CurseForge (coming soon)

CurseForge support is coming soon. Its API (`api.curseforge.com/v1`) needs a paid key, so for now the launcher
doesn't use it: the Mods page shows **CurseForge (coming soon)** beside the Modrinth switch, disabled, and no search
or install goes to CurseForge (`browser::sources()` is Modrinth only; `horizon-launcher mods ... --source curseforge`
says it is coming soon). The CurseForge source (`browser/curseforge.rs`) is kept for when it comes.

A CurseForge modpack zip or instance the player already has still imports, since that needs no key: its jars come
from the player's installed instance or, when Modrinth has the same file, from Modrinth. A jar only CurseForge serves
is listed under the pack as "CurseForge downloads coming soon".
