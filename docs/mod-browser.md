# The launcher's mod browser

Search Modrinth (and CurseForge, once the launcher has a key) for packs Horizon loads, see how well each one runs,
and install it with its checksum checked. This is item 16 of the roadmap and issue #577. The code is in
`launcher/src/browser.rs` and `launcher/src/browser/`.

## What it offers

Only what Horizon actually loads is offered. Java mod jars (Fabric, NeoForge, Forge) don't run in Horizon
([mods.md](mods.md), [research/java-mod-compat.md](research/java-mod-compat.md)), so they are never searched.
Horizon `.hmod` mods aren't on Modrinth or CurseForge yet (Modrinth has no `horizon` loader).

| Kind | Goes to | How well it runs |
|---|---|---|
| Resource packs | `resourcepacks/` beside `options.txt` | **Works** when made for 26.3 ([resource-packs.md](resource-packs.md) lists what Horizon reads). Older packs are **Partly**: they load with vanilla's compatibility warning |
| Shader packs (Iris, OptiFine) | `shaderpacks/` | **Partly**: translated to WGSL, the full-screen passes draw and the world and shadow passes don't yet ([shader-packs.md](shader-packs.md)). Packs for other loaders (Canvas, core-shader packs) **don't run** and can't be installed |
| Modpacks (`.mrpack`, CurseForge zip) | the Modpacks page's import | **Partly**: Horizon runs the mods it can port, from the player's own downloads ([modpacks.md](modpacks.md)) |

By default a search shows only what runs: resource packs made for 26.3, shader packs for Iris or OptiFine, and every
modpack. **Show everything** lists the rest of the kind too, each marked with how well it runs; what doesn't run is
never offered for install.

## Checks on every download

- The file must come from Modrinth's or CurseForge's file servers (`cdn.modrinth.com`, `edge.forgecdn.net`,
  `mediafilez.forgecdn.net`), over HTTPS.
- Its name must be one plain file name (no folders, `..`, hidden names or reserved characters).
- It is written as `.<name>.part`, hashed as it arrives, and renamed into place only when its size and checksum match
  what the source lists: SHA-512 on Modrinth, SHA-1 on CurseForge (the strongest hash CurseForge publishes). Anything
  else removes the part file and installs nothing.
- A file whose source lists no checksum is refused.

## The Mods page

The launcher's **Mods** page has the three kinds as tabs, a search field (Enter searches), **Show everything** and,
once CurseForge is set up, a Modrinth / CurseForge switch. Each result is an index row: its summary, author,
downloads, source and licence, how well it runs (and why), **Page** (the project's page on its source) and
**Install** (**Import** for a modpack). Installed packs are listed above the results with their version, source,
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
- **A modpack** is downloaded (checked the same way) into `ports/downloads/` and imported on the Modpacks page,
  which says what of it Horizon runs and ports it.

From a terminal (or an agent):

```sh
horizon-launcher mods search faithful                 # --kind resource|shader|modpack, --everything, --source
horizon-launcher mods install faithful-32x            # id or slug; a modpack is imported
horizon-launcher mods list                            # --json
horizon-launcher mods update [faithful-32x] [--check]
horizon-launcher mods remove faithful-32x
```

## Sources

**Modrinth.** The public API (`api.modrinth.com/v2`) needs no key. The launcher names itself in its user agent, as
Modrinth asks.

### CurseForge

CurseForge's API (`api.curseforge.com/v1`) needs an API key from its developer console. Ludovic approved CurseForge
for the browser on 2026-10-21; the key is his (agents never sign up for him). Until a key exists the CurseForge source
is off and the browser searches Modrinth only.

To turn it on:

1. Ludovic gets a key at <https://console.curseforge.com/> (sign in, API keys). CurseForge's terms for a launcher
   that ships the key apply: the key is built into the binary, so anyone with the binary can read it.
2. He stores it in the 1Password **Agents** vault as an item `CurseForge API`, field `credential`.
3. It goes into the repository secret `HORIZON_CURSEFORGE_API_KEY`
   (`~/fleet/op read "op://Agents/CurseForge API/credential" | gh secret set HORIZON_CURSEFORGE_API_KEY`).
   The launcher workflow passes it to the build, which builds it in (`option_env!`).

At run time `HORIZON_CURSEFORGE_API_KEY` in the environment overrides the built-in key; empty means none. A file
whose author allows downloads from CurseForge's own site only (no `downloadUrl` in the API) is shown but can't be
installed.
