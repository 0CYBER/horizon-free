# Horizon mods

Horizon gets its own mod system, written in Rust and run in a sandbox, plus
two ways to bring over what Java players use: native Horizon ports of popular
mods and the data-driven parts of packs and mods. Fabric and NeoForge jars do
**not** run as they are: their code calls and patches Minecraft's Java classes, and
Horizon has none. What carries over is the data and assets inside a jar
([modpacks.md](modpacks.md)).

Mods are **off and absent by default**. A fresh install has no `horizon-mods`
folder, loads no runtime and plays exactly like vanilla 26.3.

| Part | Where | State |
|---|---|---|
| The contract (WIT) | `crates/horizon-mod-sdk/wit/horizon.wit` | 0.1: log, game-read, chat, hud, storage; tick, frame, chat and command hooks |
| SDK for mod authors | `crates/horizon-mod-sdk` | 0.1 |
| Example mod | `crates/horizon-mod-sdk/examples/coords-hud` | coordinates, facing and time HUD, `/where`, `/coords` |
| Runtime | `crates/horizon-mods` | wasmtime 36 LTS, component model |
| Authoring tool | `crates/cargo-horizon-mod` | `cargo horizon-mod new [--template code\|content] / build / run / check`, and the registry's CI (`registry build / review / list`) |
| In the game | `crates/horizon-client/src/game/mods.rs`, Options > Mods | client mods |
| Launcher mod browser (Modrinth, CurseForge with a key) | [mod-browser.md](mod-browser.md) | resource packs, modpacks |
| Registry index (`index.json`, `.hpack`), signatures | `horizon_mods::index`, [api/mod-index.md](api/mod-index.md) | schema 1; launcher and website next |
| Server mods, registries | this page, [Roadmap](#roadmap) | planned |

## Design goals

1. **Safe to install.** A mod is untrusted code. It runs in a WebAssembly
   sandbox with no file, network, process or clock access, a memory cap and a
   time budget per call. It can only do what its capabilities grant, and the
   player sees those before turning it on.
2. **One file for every platform.** The same `.hmod` runs on Windows, macOS
   and Linux, x86-64 and ARM: authors build once, players never meet "wrong
   build for my OS".
3. **Free when unused.** No mod on: no engine, no thread, no compile, one
   branch per hook. Measured below.
4. **Vanilla stays vanilla.** Mods add; they cannot patch the game's code the
   way Mixins do. What they may change is a list of hooks the game calls at
   fixed points, so the parity harnesses keep testing the same game.
5. **Stable for authors.** The contract is a versioned WIT package; a mod
   built for API 0.1 keeps loading on later 0.1.x games.

## How it works

```
 mod crate (Rust)                         Horizon
 ───────────────                          ───────
 impl Mod for MyMod { ... }               horizon-mods::Mods
 export_mod!(MyMod)                         ├─ Engine (cranelift), made on first mod
        │ cargo horizon-mod build           ├─ one Store per mod: 64 MiB, fuel per call
        ▼                                   ├─ Linker: log, game, chat, hud (WIT imports)
 my_mod.hmod = zip {                        └─ calls the events export: init, tick,
   horizon-mod.toml,                             frame, command, chat
   mod.wasm  (a WebAssembly component)
 }
```

- **WebAssembly components.** Mods compile to `wasm32-unknown-unknown`;
  `cargo horizon-mod build` wraps the module into a
  [component](https://component-model.bytecodealliance.org/) for the
  `horizon:mods/horizon-mod` world. The WIT file is the whole API: host
  functions the mod imports (`log`, `game`, `chat`, `hud`) and the `events`
  interface it exports. No WASI: the component cannot even name a file or a
  socket.
- **Capabilities are imports.** Every imported interface except `log` is a
  capability. The manifest declares them; the game reads the component's
  actual imports and refuses a mod whose code imports one its manifest does not
  declare (`uses capability chat, missing from horizon-mod.toml`). So the list
  the player reads is the truth, not a promise.
- **Hooks on request.** `init` returns a `registration`: the commands the mod
  owns and which of `tick`, `frame` and `chat` it wants. The game calls only
  those. A mod that wants nothing per frame costs nothing per frame.
- **Retained HUD.** A mod sets its panel (corner and coloured lines) when it
  changes; the game draws it every frame with the extras' overlay code. Most
  HUD mods need no frame hook at all.
- **Snapshots, not callbacks into the world.** Once per tick (and before a
  command) the game copies what mods may read (player position, look, health,
  food, dimension, world clock) into a snapshot shared by all mods. Mods never
  hold references into the game, so they cannot stall or corrupt it.

### What a mod can do (API 0.1)

| Interface | Capability | Functions |
|---|---|---|
| `log` | always | `write(level, message)`, in the game log as `[mod <id>]` |
| `game` | `game-read` | `get-player()`: x y z, yaw, pitch, health, food, dimension; `get-clock()`: game time, day time |
| `chat` | `chat` | `show(text)`: a line only the local player sees, after the mod's name (8 per call) |
| `hud` | `hud` | `set-panel(corner, lines)`, `clear()`: up to 16 lines of 256 characters |
| `storage` | `storage` | `get(key)`, `set(key, value)`, `delete(key)`: a per-mod folder of up to 64 files, 4 MiB total, at most 1 MiB written per call |

| Hook (export) | When |
|---|---|
| `init() -> result<registration, string>` | once, after loading; an error keeps the mod off and is shown |
| `tick(tick)` | each game tick while in a world (20 per second) |
| `frame(seconds)` | each rendered frame |
| `command(name, args) -> result<option<string>, string>` | `/name args` typed in chat, for a command the mod registered |
| `chat(text)` | each chat line the player receives, as plain text |

Commands: vanilla's command names win. A mod asking for `/give` does not get
it (logged), and two mods cannot share a name (the first one loaded keeps it).
The chat box colours and Tab-completes a mod's commands like vanilla's, for
every player whatever their permission level, and takes any text after the
name (the mod reads it as `args`; API 0.1 has no way to declare argument types).

### Planned interfaces

Each is a new capability, added in a 0.x minor version so older mods keep
loading. Ordered by what the first native ports need:

| Interface | For | Notes |
|---|---|---|
| `keys` | keybinds | registered binds appear in Controls; press/release events |
| `target` | the crosshair's block or entity, with its state and NBT summary | Jade/WTHIT-style tooltips |
| `inventory-read` | the player's inventory, held item, open container | AppleSkin, recipe viewers |
| `recipes` | the recipe registry and lookups | JEI/EMI-style viewers |
| `ui` | widgets: text, item icons, lists, a screen the mod owns, a settings page in Options > Mods | built from vanilla's widgets (`ui`) so it matches the menus |
| `world-read` | blocks and biomes around the player, within the render distance | minimaps, schematics |
| `render` | world-space lines, boxes and ghost blocks in a per-frame budget | Litematica-style placement, waypoints |
| `registries` | new blocks, items, entities and recipes, mostly data: JSON models, textures, loot and recipes in the `.hmod`'s `assets/` and `data/` | server side; see [Multiplayer](#multiplayer) |
| `server` | world events (block break/place, entity spawn/death, player join), game rules, server commands | runs where the world is simulated |
| `net` | a mod's own client↔server channel (custom payloads) | only between the two halves of the same mod |

There is no file, socket, process or raw-input interface, on purpose.

## Choosing the runtime

| Option | Sandbox | One file for all OSes | Typed API | Cost | Verdict |
|---|---|---|---|---|---|
| **wasmtime + component model** | yes: memory, fuel, no imports but ours | yes | WIT → Rust on both sides | 85 ns per empty hook call; 50 ms to compile a 64 KB mod on load | **chosen** |
| wasmtime, core modules with a hand ABI | yes | yes | no: pointers and lengths by hand | a few ns less per call | not worth the fragile ABI |
| wasmi (interpreter) | yes | yes | no components | no compile time, execution ~10-50x slower than cranelift code | kept in mind for a platform without JIT |
| Lua (mlua) or another script | partial (strip the stdlib, hope) | yes | no | fast to write | not Rust; weak sandbox |
| native `cdylib` plugins | none | no: per OS and arch, unstable Rust ABI | yes | none | unsafe for players |
| an embedded JVM | none per mod | yes | Java | 100+ MB, seconds of startup | see the research |

wasmtime 36 is a long-term-support line (security fixes until August 2027)
that builds on the client's Rust 1.90. Only its `runtime`, `cranelift`,
`component-model` and `std` features are on: no WASI, no async, no GC
proposal, no profiling. Its cranelift backend generates native code for x86-64
and ARM64 on Windows, macOS and Linux, which is why one `.hmod` serves them
all.

The game links it only with its `mods` feature: every release build turns it
on (`release.yml`), and a build from source needs
`cargo build --release -p horizon-client --features mods` to run code mods.
Without it (the tests and probes) wasmtime and cranelift aren't built, a
quarter of a clean build; content mods still load, and the game logs why the
code mods don't start.

### Performance

Measured on the Mac mini (M4 Pro), release build, `cargo test -p
horizon-mods --features runtime,pack --release --test sandbox -- --ignored --nocapture`:

| | per call |
|---|---|
| tick with no mod loaded | 1.2 ns (one check) |
| empty `tick` hook (call into the sandbox and back, fuel on) | 85.5 ns |
| empty `frame` hook | 87.5 ns |
| coords-hud's `tick` (reads the player and clock, formats 3 lines, compares) | 2.3 µs |
| loading coords-hud (read, compile with cranelift, instantiate, `init`) | 50 ms, once |

In the game, same world and view, `horizon-ctl bench 10` three times each
(4 other sessions shared the machine, so the tails are noisy; the median is
the stable figure):

| | median frame | mean FPS |
|---|---|---|
| no `horizon-mods` folder | 2.62-2.71 ms | 149-163 |
| coords-hud on | 2.72-2.78 ms | 151-254 |

At 20 ticks per second coords-hud costs 46 µs per second of game time: well
under 0.01 ms per frame. The frame-time medians differ by less than the
run-to-run spread.

**Budgets.** Each call gets fuel (about one unit per WebAssembly instruction):
2 billion for `init`, 50 million for `tick`, 10 million for `frame`, 100
million for a command or chat line. A runaway loop ends in a fraction of a
second (the sandbox test's endless `/spin` is cut well within its 5 s limit)
instead of hanging the game. The target budgets a mod should stay under are
tighter: **1 ms per tick and 0.1 ms per frame**. The runtime keeps each mod's
average call time (`Entry::cost`); the Mods page will show it, and a mod over
budget for long will be flagged, then paused.

### Crash isolation

A trap (panic, out-of-bounds access, `unreachable`), running out of fuel or
hitting the 64 MiB memory cap stops **that mod only**. The game says so in
chat (`Mod Test Guest stopped: it took too long (an endless loop?)`), marks it
on the Mods page and carries on; the other mods keep running. Host functions
clip what a mod hands back (chat lines per call, HUD lines and lengths). Tests:
`crates/horizon-mods/tests/sandbox.rs` (an endless loop, a panic and a 1 GiB
allocation, next to a healthy mod; `cargo test -p horizon-mods --features
runtime,pack`).

## Playing with mods

1. Put `.hmod` files (or mod folders holding `horizon-mod.toml` and
   `mod.wasm`) in `horizon-mods/` beside `options.txt` (in the launcher's game
   folder).
2. Options > **Mods** lists them with what each may do ("May see your
   position, health and the time, draw text on your screen, ...") and a
   switch. They start off; the choice is kept in `horizon-mods.txt`
   (`mod.<id>:true`).
3. A mod that failed to load or stopped shows why, in red, on its row.

Two turned-on mods never share an `id` while running (it is also capability
`storage`'s directory): the one `find` lists first starts, the other stays
off with the reason on its row.

Benchmarks (`--benchmark`) never load mods.

## Making a mod

```sh
rustup target add wasm32-unknown-unknown
cargo install --git https://github.com/ludovic111/horizon cargo-horizon-mod
cargo horizon-mod new my-mod        # Cargo.toml, horizon-mod.toml, src/lib.rs
                                    # (--template content: a content mod, no code)
cd my-mod
cargo horizon-mod run               # build, then start the game with it
cargo horizon-mod build             # again after an edit: the running game reloads it
```

`run` starts `horizon-client` (on `PATH`, or `--game PATH`, or
`HORIZON_CLIENT`) with `--dev-mod target/horizon-mod/<id>.hmod`. A dev mod is
always on, and the game checks its file once a second: rebuild and it restarts
with "Mods reloaded" in chat. Its in-memory state starts over; whatever it
saved with capability `storage` survives the reload.

Before you submit it to the registry, run what its CI runs:

```sh
cargo horizon-mod check target/horizon-mod/my_mod.hmod   # --json for the report as JSON
```

It reads the manifest and content, refuses imports the capabilities don't
cover, then plays the mod in the sandbox for 40 ticks, 120 frames, a chat line
and each of its commands: it must start and never trap or overrun. It checks a
`.hpack` and a resource pack `.zip` too, and a signature with `--key <public
key> --signature <file>.minisig`.

### Template layout

```
my-mod/
├── Cargo.toml           # cdylib, horizon-mod-sdk, release: opt-level "s", LTO
├── horizon-mod.toml     # the manifest
└── src/lib.rs           # impl Mod, export_mod!
```

A mod is a type implementing `horizon_mod_sdk::Mod`; its state lives in the
type:

```rust
use horizon_mod_sdk::{Mod, Registration, chat};

struct Hello { greeted: u32 }

impl Mod for Hello {
    fn init() -> Result<(Self, Registration), String> {
        Ok((Hello { greeted: 0 }, Registration::new().command("hello")))
    }
    fn command(&mut self, _name: &str, args: &str) -> Result<Option<String>, String> {
        self.greeted += 1;
        chat::show(&format!("Hello {args}! ({} so far)", self.greeted));
        Ok(None)
    }
}
horizon_mod_sdk::export_mod!(Hello);
```

Hooks not asked for keep their default bodies. The full example is
[`coords-hud`](../crates/horizon-mod-sdk/examples/coords-hud/src/lib.rs). Logic
that does not touch the API (formatting, maths) is ordinary Rust and unit-tests
on the host with `cargo test`.

### The manifest

```toml
id = "coords_hud"            # [a-z0-9_]{1,64}: settings key, log name
name = "Coordinates HUD"
version = "0.1.0"
description = "..."
authors = ["..."]
api = "0.1"                  # the API the SDK targets
side = "client"              # client | server | both
capabilities = ["game-read", "hud", "chat"]
```

Unknown keys are refused, so a typo is an error, not a silently ignored
permission.

### Content mods (API 0.2)

A mod with `api = "0.2"` and capability `registries` may carry `assets/` and
`data/` in the `.hmod`, and needs no `mod.wasm` if it has no code
(`cargo horizon-mod new --template content` starts one; `build` in a folder
with no `Cargo.toml` packs the manifest, `assets/` and `data/`, and `check`
reads it without a trial run). It adds
blocks and items by declaring them in `data/<id>/horizon/block/*.json` and
`data/<id>/horizon/item/*.json`, each copying a vanilla template
([modpacks.md](modpacks.md#registries-02-data-driven-blocks-and-items)):

```json
{ "template": "minecraft:iron_block",
  "properties": { "hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block" },
  "item": true }
```

The fence: files go in the mod's own namespaces (`assets/<id>/`,
`data/<id>/`, and those its manifest lists), plus additions to vanilla tags
(`data/minecraft/tags/`) and to the convention tags NeoForge and Fabric mods
share (`data/c/tags/`).
Templates and `sounds` name vanilla blocks; unknown keys are refused.

Some Java mods ship their files under other names than their mod id (Towns
and Towers, mod id `t_and_t`, under `kaisyn` and `towns_and_towers`). A
content mod lists those in its manifest:

```toml
id = "t_and_t"
api = "0.2"
capabilities = ["registries"]
namespaces = ["kaisyn", "towns_and_towers"]
```

A listed namespace is the mod's as its id is: its `assets/` and `data/`
files load, and the game reads its tags, recipes, names and loot. Blocks and
items are still declared under `data/<id>/horizon/` only. `minecraft`, `c`
and `horizon` cannot be listed. Two mods that are on may not own the same
namespace (one's id or listed namespace being the other's): the game keeps
the first in load order and leaves the other's content out, with
"mods A and B both use the namespace N; turn one of them off" in the log. Paths
use vanilla's resource characters, at most 65,536 files, 16 MiB each and
512 MiB in all. A mod whose content is refused stays off, with the reason on
its row. The game reads this content into its block and item tables
(`horizon_client::mod_content`, docs/modpacks.md). A tag entry naming an id
nobody registers (a block a port left out, another mod's item) is dropped,
so the rest of the tag still loads; vanilla would drop the whole tag.

Tags merge as a data pack's do. Every mod's file for one tag (vanilla's
`data/minecraft/tags/`, the convention `data/c/tags/`, or another mod's) adds
its values after vanilla's and the mods' before it, in load order. A file with
`"replace": true` first drops what came before it, vanilla's entries and the
earlier mods', and the mods after it still add theirs, as a higher data pack
does. Since the result depends on load order, the game says so in the log:
"mod towns replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json:
vanilla's entries and those of adds are left out, ...". Tags are the only
`minecraft` files a mod may hold: its other content stays in its own
namespaces.

A mod adds to structure sets and template pools that are not its own
(vanilla's villages, another mod's houses) with a set or pool file in its own
namespace holding `horizon:add_to`, a key vanilla data never has. Its entries
are appended to each set or pool named, after every mod's own files, in mod
and file order; the file is no set or pool itself. A name nobody defines is
skipped.

```json
{ "horizon:add_to": "minecraft:villages",
  "structures": [ { "structure": "mymod:village_beach", "weight": 10 } ] }
```

A pool addition holds `"elements"` as a pool does. This is what lithostitched's
`add_structure_set_entries` and `add_template_pool_elements` modifiers do on
NeoForge and Fabric, and `horizon-port-pack carry` writes them from those.

A structure (and any of its pool elements, in place of the structure's) may
hold `horizon:enhanced_terrain_adaptation`: YUNG's API's and
MoogsStructureLib's terrain kernel, which carves or buries terrain around the
pieces on top of vanilla's `terrain_adaptation`. `kernel_size` and
`kernel_distance` shape the kernel; `top` and `bottom` (`carve`, `bury` or
`none`) act above and below a piece's floor; `bottom_offset`, `padding`
(`x`, `top`, `bottom`, `z`) and MoogsStructureLib's `band` (`bottom`, `top`,
`piece_heights`) move the box it measures from. On a structure,
`rigid_junctions` (YUNG's API) adds rigid pieces' junctions as well and
`inflate` grows the start's box by the kernel radius. `kernel_size` 0 is
none.

```json
{ "horizon:enhanced_terrain_adaptation": { "kernel_size": 24, "kernel_distance": 24,
    "top": "carve", "bottom": "none", "rigid_junctions": true, "inflate": true } }
```

## Publish your mod

Horizon's registry lists mods, modpacks (`.hpack`) and resource packs made for Horizon, each reviewed before it is
published. Players find them at [playhorizon.dev/mods](https://playhorizon.dev/mods) and in the launcher (Mods, then
**Horizon** beside Modrinth), which checks every file before it installs it.

1. Run `cargo horizon-mod check` on the file (above): the registry runs the same checks.
2. Put the source somewhere public, under a licence that lets players run it: reviewers read it, and the file must be
   built from it.
3. Open [playhorizon.dev/mods/submit](https://playhorizon.dev/mods/submit), sign in with GitHub (it reads only your
   public profile) and fill in the page: the file, its name and summary, licence, source address, and optionally a
   minisign public key with the file's `.minisig`.
4. The form checks the file again and opens a pull request in
   [ludovic111/horizon-mods](https://github.com/ludovic111/horizon-mods) that adds `mods/<id>.toml`, with you as its
   owner. That repository's checks run the file in the sandbox and post their report on the pull request; answer the
   reviewers there.
5. A reviewer merges it: within minutes the file is a release of horizon-mods, and the project is on the site and in
   the launcher.

An update is the same form with a new version; only the project's `owners` can publish one, signed with its key if it
has one. A published file never changes. Without the form, open the pull request by hand (the repository's README
says how). The formats: [api/mod-index.md](api/mod-index.md).

## Versions and compatibility

- The API is the WIT package `horizon:mods@0.1.0`. New functions and
  interfaces arrive in patch versions (0.1.1, ...); a game serving 0.1.N loads
  mods built for 0.1.0 to 0.1.N. A breaking change makes `0.2` (and later
  `1.0`); the game then links both versions side by side for at least one
  release cycle, so existing mods keep working while authors move.
- The manifest's `api` names the line it needs (`0.1`); the game refuses a
  line it does not serve with a clear message.
- Horizon follows Minecraft releases; the mod API does not have to. Vanilla
  concepts mods see (dimensions, block and item ids) are the game's own ids,
  so a version port changes the data, not the API.

## Multiplayer

- **Client mods** (`side = "client"`, all of 0.1) run in the player's game,
  in single player and on any server, vanilla or Horizon. They see only what
  the client already knows (the snapshot), like F3 does, and act only on the
  local screen and chat. A server cannot see them.
- **Server mods** (`side = "server"`) will run where the world is simulated:
  Horizon's integrated server in single player and LAN, and the dedicated
  server. Vanilla clients can join if the mod only uses vanilla content
  (rules, commands, events).
- **Both** (`side = "both"`): a server mod with a client half (new blocks,
  their textures, a custom screen). On join the Horizon server announces its
  mods (id, version, SHA-256) on a `horizon:mods` custom payload channel; a
  Horizon client with matching client halves enables them for that session, or
  offers to download them from the same source the launcher uses. Vanilla
  clients are refused only if the server says its mods need a client.
- Registries (new blocks and items) are server-owned and synced like vanilla's
  registry sync, so ids match on both ends.

## Distribution

- **The file.** `.hmod`: a zip holding `horizon-mod.toml`, `mod.wasm` and,
  later, `assets/` and `data/`. `mod.wasm` is capped at 32 MiB.
- **Integrity.** Every download is checked against the SHA-256 the index
  lists before the file reaches `horizon-mods/`.
- **Signing.** Authors sign `.hmod` files with an Ed25519 key (minisign
  format: a `.hmod.minisig` beside it); the index pins each project's public
  key, so an update signed by another key is refused. Unsigned local files
  stay allowed, with an "unsigned" mark on the Mods page.
- **Where mods come from.** Horizon's registry ([Publish your mod](#publish-your-mod)): reviewed pull requests in
  [ludovic111/horizon-mods](https://github.com/ludovic111/horizon-mods), files as its release assets, and an
  `index.json` on GitHub Pages that the launcher and the website read. A file the registry takes down
  (`revoked.toml`) is refused by the launcher and turned off where installed.
- **The launcher.** A Mods tab per installation: browse and search the index,
  install, update, turn on and off (it writes the same `horizon-mods.txt`),
  see capabilities before installing. The launcher's CLI and MCP tools gain
  `mods list|search|install|remove|enable|disable`, so agents can set up a
  modded game the same way players do.

## Roadmap

1. Design (this page) and the Java research.
2. SDK, example mod, runtime, `cargo horizon-mod`, the game's Mods page: API 0.1.
3. `storage` (done), `keys`, `target`, `inventory-read`, `ui`: enough for the
   first native ports (zoom presets, AppleSkin-style food HUD, Jade-style
   tooltips).
4. Launcher Mods tab, the index, signatures; the Mods page shows cost and
   update state.
5. `recipes`, `world-read`, `render`: recipe viewer and schematics ports.
6. Server mods, `registries`, `net`, data-driven content from `.hmod`
   `data/` and `assets/`, multiplayer sync.

Modpacks ([modpacks.md](modpacks.md)) move data-driven `registries` first:
API 0.2 is content-only `.hmod`s (blocks, items, recipes, loot, models and
textures), then server hooks (0.3) and block entities, menus, capabilities and
`net` (0.4), in the order the ATM10 inventory needs them.
