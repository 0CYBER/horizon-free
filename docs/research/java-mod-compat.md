# Java mods and Horizon: what can carry over

Players ask "will my mods work?". Short answer: **Fabric and NeoForge jars
will not run in Horizon as they are**, and no bridge we could build would
change that for more than a handful of mods. What carries over is the
*content* (data packs, resource packs, the data and assets inside mod jars)
and the *ideas* (native Horizon ports of the mods people use most, written
with [Horizon's own mod system](../mods.md)). This page is the reasoning.

## How Java mods work

### Fabric

- **Fabric Loader** finds jars in `mods/`, reads `fabric.mod.json`
  (entrypoints, dependencies, mixin configs, access wideners), and starts the
  game through a class loader that can rewrite classes as they load.
- **Mixin** (SpongePowered) is how nearly every mod changes the game: a mixin
  class names a Minecraft class (`net.minecraft.client.gui.hud.InGameHud`) and
  injects code at points inside its methods (`@Inject(method = "render", at =
  @At("TAIL"))`, `@Redirect`, `@ModifyVariable`, `@Overwrite`...). The bytecode
  of the vanilla class is rewritten at load time. Access wideners make private
  fields and methods public for the same purpose.
- **Fabric API** is a set of modules that give mods events and registries
  instead of raw mixins: `fabric-lifecycle-events` (`ClientTickEvents`,
  `ServerTickEvents`), `fabric-rendering` (`HudRenderCallback`,
  `WorldRenderEvents`), `fabric-command-api`, `fabric-key-binding-api`,
  `fabric-networking-api` (custom payloads), `fabric-registry-sync`,
  `fabric-resource-loader`, `fabric-screen-api`, `fabric-item-group`,
  `fabric-events-interaction` and about forty more. **Fabric API itself is
  implemented with mixins** into the same vanilla classes.
- Mods mix the two freely: a mod using Fabric API events still calls
  `MinecraftClient.getInstance().player.getBlockPos()`,
  `world.getBlockState(pos)`, `DrawContext.drawText(...)`: direct calls into
  vanilla's classes, by their mapped names.

### NeoForge (and Forge)

- An event bus (`NeoForge.EVENT_BUS`, mod buses) with hundreds of events,
  `DeferredRegister` for registries, data attachments and capabilities,
  networking payloads, config specs. NeoForge patches vanilla's source
  directly (binary patches) to fire its events.
- Mods also use mixins and access transformers, and call vanilla classes
  directly, as on Fabric.

### Data and assets in a jar

Besides code, a mod jar carries the same folders as packs:
`assets/<namespace>/` (models, blockstates, textures, sounds, language files,
shaders) and `data/<namespace>/` (recipes, loot tables, tags, advancements,
predicates, functions, worldgen: biomes, features, structures). These are
**vanilla formats**, read by vanilla's own code.

## Why a Java bridge cannot run them

Horizon is a reimplementation in Rust: there are no `net.minecraft` classes
at runtime, no `InGameHud`, no `ClientPlayerEntity`, no bytecode to rewrite.

1. **Mixins have nothing to bind to.** A mixin is a patch against a specific
   method's bytecode. Without vanilla's classes there is no method, no
   injection point and no local variable to modify. This is not a missing
   feature we could add: it is the mechanism itself.
2. **Fabric API is mixins too.** A shim Fabric API would have to be
   re-implemented from scratch as Java classes calling into Horizon. That part
   alone is possible for the event modules.
3. **Mods call vanilla directly.** Even "pure Fabric API" mods use vanilla's
   classes for everything else: the player, the world, the block states, the
   renderer, the text drawing. Running them means providing Java classes with
   vanilla's names and behaviour, backed by Horizon's state: in effect a
   second Minecraft, in Java, on top of Horizon. Thousands of classes, in the
   mapped names of every mapping set (Yarn, Mojang, intermediary), version by
   version.
4. **The JVM itself** would ship with every install (a trimmed JRE is still
   40-60 MB, the full one 150+ MB), start in a second or more, hold a few
   hundred MB of heap before any mod loads, and bring garbage-collector pauses
   back into the frame: the costs Horizon exists to remove. JNI calls cost
   tens to hundreds of nanoseconds each and every one crosses with copies.
5. **No sandbox.** Java mods are fully trusted native-speed code with file
   and network access. Horizon's mods are sandboxed and their permissions are
   shown; a Java bridge would be the one hole in that.

**What a bridge could support**, if we ever built one: a stable subset of
Fabric API events (client and server tick, HUD render callback, client
commands, key bindings, custom payloads), with a tiny "vanilla view" API
(player position, held item, chat), for mods written only against that
subset. Surveying the most-downloaded Fabric mods, nearly none qualify: the
popular ones are popular precisely because they reach into rendering, the
world or the UI. The cost (a JVM in the game, a second API surface to keep
stable) is far above the benefit. **We do not build it.** If that changes,
the bridge would sit on top of the WIT API like any other mod runtime, so the
sandbox would still hold.

## What we do instead

### 1. Data-driven content (packs and the data in mod jars)

Horizon already reads vanilla resource packs and data packs (functions,
function tags, predicates today). Extending data-pack coverage to recipes,
loot tables, tags, advancements and worldgen JSON brings most "content"
packs over with no mod code at all, and it is vanilla behaviour, so the parity
harnesses check it.

For **Fabric/NeoForge jars**, the launcher can offer "import the data": read
`data/` and `assets/` from the jar and load them as a data pack plus a
resource pack. What works: new recipes, loot changes, tags, advancements,
functions, worldgen tweaks that only use vanilla blocks, retextures, language
files. What does not: anything referring to a block, item or entity the jar
registers in Java code (the ids do not exist); the import skips those files and
lists them. With Horizon's planned `registries` interface, a native port can
register those ids and reuse the jar's JSON models, textures, recipes and loot
as they are.

### 2. Native Horizon ports of popular mods

Horizon itself covers the performance mods: Sodium, Lithium, FerriteCore,
ModernFix, EntityCulling and ImmediatelyFast exist because vanilla Java is
slow, and Horizon's renderer and simulation are the replacement (see
[benchmark](../benchmark.md)). Zoom (OK Zoomer, Zoomify), FPS and keystroke
overlays, a minimap and auto-reconnect are already built in as
[extras](../extras.md).

#### Port first

Ranked by players reached per unit of API work. Each needs the planned
interfaces listed in [mods.md](../mods.md#planned-interfaces).

| Mod (Java) | What it does | Horizon port | Needs |
|---|---|---|---|
| **Mod Menu** | list mods, open their settings | built in: Options > Mods (done), settings pages next | `ui` |
| **AppleSkin** | food saturation and exhaustion on the HUD and tooltips | SDK mod | `inventory-read`, `ui` |
| **Jade / WTHIT** | what the crosshair points at: block, entity, harvest tool, container contents | SDK mod | `target`, `ui` |
| **JEI / EMI / REI** | browse items, recipes and uses | SDK mod (or built in: Horizon already has the recipe book's data) | `recipes`, `inventory-read`, `ui` |
| **Xaero's Minimap / JourneyMap** | minimap, world map, waypoints | the minimap is an extra; a world map and waypoints as an SDK mod | `world-read`, `render`, `storage` |
| **Litematica** | schematic overlay to build from | SDK mod | `world-read`, `render`, `storage`, `ui` |
| **Mouse Tweaks / Inventory Profiles Next** | drag and sort inventory | built in or SDK mod | `inventory-read` + a sanctioned click API |
| **Continuity / CIT** | connected and custom item textures | built into the renderer (pack formats) | none: resource-pack features |
| **Distant Horizons** | low-detail far terrain | a Horizon renderer feature, not a mod | none |
| **Iris + shader packs** | shaders | a Horizon renderer feature (wgpu shader pipeline), later | none |
| **Simple Voice Chat** | proximity voice | server + client mod | `server`, `net`, audio (later) |

### 3. A porting guide (outline)

To be written as `docs/porting-from-fabric.md` once API 0.2 lands:

1. **Separate logic from glue.** Most of a mod is plain logic (maths,
   formatting, data structures): port it to Rust once, unit-test it on the
   host.
2. **Map events.**

   | Fabric | Horizon |
   |---|---|
   | `ClientModInitializer.onInitializeClient` | `Mod::init`, returning a `Registration` |
   | `ClientTickEvents.END_CLIENT_TICK` | `Registration::on_tick`, `Mod::tick` |
   | `HudRenderCallback` | `hud::set_panel` when the content changes (retained), or `on_frame` |
   | `ClientCommandRegistrationCallback` | `Registration::command`, `Mod::command` |
   | `ClientReceiveMessageEvents` | `Registration::on_chat`, `Mod::chat` |
   | `KeyBindingHelper.registerKeyBinding` | `keys` (planned) |
   | `Registry.register(Registries.ITEM, ...)` | `registries` (planned) with JSON in `data/` |
   | `ServerPlayNetworking` payloads | `net` (planned) |
   | config libraries (Cloth Config, YACL) | `storage` + a `ui` settings page (planned) |

3. **Replace mixins with hooks.** For each mixin, write down what it observes
   or changes. If a WIT hook covers it, use the hook. If not, open an issue
   asking for the hook ("I need X before/after Y"): hooks are added to the API
   for everyone instead of each mod patching the game.
4. **Declare capabilities** in `horizon-mod.toml`, the least the mod needs.
5. **Reuse assets**: textures, models, language files and recipes move as
   they are into the `.hmod`'s `assets/` and `data/` (with `registries`).
6. **Test** with `cargo horizon-mod run` and hot reload; add unit tests for the
   logic; publish with a signed `.hmod`.

## Sources

- Fabric documentation (`docs.fabricmc.net`): mod structure, Mixin
  introduction, Fabric API modules and events.
- SpongePowered Mixin wiki: injection points and how class transformation
  works.
- NeoForge documentation (`docs.neoforged.net`): events, registries,
  networking.
- Modrinth's most-downloaded Fabric mods, for the porting list.
- Horizon's own Java test mod (`mods/horizon`, built by `tools/build_mod.py`
  and the CI `portable-mod` job) is a Fabric mod used by the benchmarks and
  parity oracles: it shows the Mixin pattern (`FrameHookMixin`) this page
  describes, and stays a developer tool, not a player-facing mod.
