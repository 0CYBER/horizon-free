# Horizon: a map for agents

Minecraft: Java Edition 26.3 rewritten in Rust (wgpu). Vanilla is the specification wherever it has the
feature; performance work may differ inside as long as it stays compatible with vanilla
([CONTRIBUTING.md](CONTRIBUTING.md#when-it-doesnt-have-to-be-11)). Read [CONTRIBUTING.md](CONTRIBUTING.md) once;
this page tells you where things are.
Feature → files → harness for the most-changed areas: [docs/architecture.md](docs/architecture.md).

## Rules that bite

- **Never commit Mojang's files**: game jars, assets, decompiled source, prepared resources (`.local/`).
  `tools/check_public_tree.py` rejects them; `git config core.hooksPath .githooks` runs it on commit.
- **Keep your runs away from real player data.** Any game or launcher you start gets fresh `mktemp -d`
  folders in `HORIZON_HOME` (logs, crash reports, telemetry consent, recordings; not saves), `HORIZON_LAUNCHER_HOME`, `XDG_DATA_HOME` and
  `XDG_CONFIG_HOME`, plus `HORIZON_LAUNCHER_NO_SELF_UPDATE=1` and `HORIZON_NO_AUDIO=1`. Saves, `options.txt` and
  caches go in the game folder, by default the working directory: start the game from a scratch folder with
  `--game-dir` pointing at it, or they land in the repo root, hidden by `.gitignore`. For the
  launcher, `HORIZON_LAUNCHER_TRASH`, `HORIZON_LAUNCHER_CLIPBOARD` and `HORIZON_LAUNCHER_DESKTOP` pointing into the scratch home. Never write to
  `~/.local/share/{applications,horizon,horizon-launcher}` or `~/.local/bin`.
- **Mark what an agent posts, once.** One mark, `🤖 Agent:`, in one place per post:
  - An issue, a comment and a review **start** with it: `🤖 Agent: <text>`. `/claim` and `/unclaim` follow it
    (`🤖 Agent: /claim`). `/nightly` and the triage commands are read from a comment's first line only, so
    there the command is the first line and `🤖 Agent:` starts the second.
  - A PR body **ends** with it, naming the model and the tool that did the work and what each did, one sentence
    per model: `🤖 Agent: Claude Fable 5.2 via Claude Code: implementation. GPT 6.1 Sol via Codex: review.`

  Nothing else is needed: no first line, no "Generated with" line, no `Co-Authored-By:` line for the agent in
  commits (main gets one squashed commit per PR, and the PR says who did the work; a co-author line for a person
  who wrote part of it stays). The person an agent works for is the account it
  posts from; write only as that person. Posts in the earlier form (a first line "Opened by <login>'s AI
  agent …", "Filed by" on issues, a last line "Generated with …") stay valid: don't rewrite them, and don't ask
  anyone to.
- **Layers** ([docs/layers.md](docs/layers.md)): engine uses engine; platform uses engine and platform; Horizon (its extras,
  cosmetics, shop, telemetry, launcher) uses platform and Horizon only, never the engine: it moves to its own repository
  later. `tools/check_layers.py` checks it. New Horizon code goes behind `horizon-extras`; built-in addresses go in
  `horizon-addresses`.
- Everything players run is Rust. Python/Java only for dev tools and harness drivers in `tools/`.
- Horizon-only features follow [docs/design-system.md](docs/design-system.md) and are off by default where
  they would change the vanilla look.
- PRs under ~3000 lines of code (docs, test files, inline test modules and moved lines don't count:
  [CONTRIBUTING.md](CONTRIBUTING.md#pull-requests)). Labels: `feature` (players get something new), `skip-release`
  (they see no change), none for a fix. Agents never add `release-now`: the maintainer decides it. Merges ship in batches of 5, or after
  6 hours (the launcher: 1 hour) ([docs/release-flow.md](docs/release-flow.md)).
- **No hardcoded user-facing strings.** Keep UI translation-ready from the start. In the launcher (`launcher/`), wrap visible text with `tr("...")` or `trf("... {arg} ...", &[("arg", &val)])` from `crate::i18n` instead of raw string literals or `format!`. In the game, use translation keys (`Text::tr`, `lang::get`), never hardcoded English strings.
- **Claim before you start**: comment `/claim` on the issue (no issue yet: open one, then claim it). An assigned
  issue is taken, and so is one labelled `in progress` (claimed on Discord): pick another. `/unclaim` when you stop;
  a claim with no open PR linking it is released after 5 days. The issues free to claim, without reading threads:
  `gh issue list --search 'is:open no:assignee -label:"in progress" -label:tracker -label:proposal -label:"not claimable" -linked:pr'`
- **Machine translations stay unreviewed.** `tools/lang_review/` records only text a person who speaks the language
  wrote or checked; never `lang_coverage.py --mark-reviewed` machine text ([docs/translations.md](docs/translations.md)).
- **Controllers reach what registers.** Draw controls with a registering helper (`ui::button`, `settings_shell::foot_button`); `graphics_style::button` and `screen_draw::button` only paint, and registering one part of a screen ends its crosshair, so register all of it.
- **Render passes keep HDR's light.** A pass that reads, filters or redraws the world's picture (upscaling,
  anti-aliasing, sharpening, frame generation, post effects, captures) follows [docs/hdr-passes.md](docs/hdr-passes.md).

## Don't lose time

Where agent time went, measured on sessions and CI in November 2026: reading code, compiling `horizon-client`,
waiting in loops, and work someone else had already done. Tests were a small part.

- **Check nobody else is on it.** One in three pull requests closed unmerged was a duplicate. Before you write
  code, look at the issue's linked pull requests and run
  `gh pr list --state open --search "<issue number, function or file name>"`; then `/claim`.
- **Find code with the map, not by reading.** Start from the tables below and
  `python3 tools/port_ledger.py where Class.method`. Read the function you change and its callers, not whole
  files.
- **Compile cheaply while you work.** `cargo check -p <crate>` after an edit. Clippy, fmt and tests run once, at
  the end, through `pr_ready.py`. `cargo clippy -p horizon-client` and `cargo build -p horizon-client` are the
  slowest commands here: never after every edit, and build the game only when you need to run it.
- **Don't wait in loops.** Start a long command in the background and carry on, or wait for it once. No
  `sleep` or `until grep` polling of builds, harnesses or CI.
- **One test per changed behaviour.** None for a refactor or a file move, none that reads source text, and no
  harness where vanilla has nothing to compare (Horizon's own features, performance work). When registering new
  `#[ignore = "daily"]` tests under `tools/ci/ignored-tests/`, prefer a focused file (`tools/ci/ignored-tests/<area>.txt`)
  rather than appending to the giant shared lists.
- **A red check isn't always yours.** When `verify` fails in a test your change can't reach, say so in the pull
  request and move on; don't hunt it.
- **Bot reviews are advice.** CodeRabbit is often right and sometimes wrong. Check each finding against the code,
  the vanilla game and these rules; fix the real ones, answer a wrong one once with the reason, and resolve the
  thread. A missing or rate-limited bot review blocks nothing.
- **Unsure about a rule? Finish the work.** Take the reading that lets you finish, and say in the pull request
  which one you took. Don't stop, and don't change direction halfway.

## Crates

Game workspace (root `Cargo.toml`); `launcher/` and `crates/horizon-telemetry-server` are separate workspaces.

| Crate | Owns |
|---|---|
| `horizon-client` | Gameplay library (`src/lib.rs`: `game.rs` + `game/`, `entry.rs`) and thin executable (`src/main.rs`). Also the dedicated server (`--dedicated`). Re-exports `horizon-client-lib` for existing mod paths. |
| `horizon-client-lib` | Reusable client systems, `horizon-ctl`, probes and their tests. Its manifest points to the existing sources under `crates/horizon-client/`; the library root is `src/client_lib.rs`. It never depends on gameplay. |
| `horizon-gameplay` | Vanilla's logical side, what the server jar holds (common code included), moved out of `horizon-client` module by module. Engine layer, not yet portable: `portable.pending` lets it take `horizon-client-lib` until the types it needs move down. No dependency on `horizon-client-gameplay` |
| `horizon-client-gameplay` | Client-only gameplay moved out of `horizon-client`: depends on `horizon-client-lib` and portable crates, never on `horizon-gameplay`. Engine layer, desktop |
| `horizon-items` | Blocks and items below gameplay: `blocks.rs` (block facts), `items.rs`, `item_data.rs` (components), `enchant.rs`, `cooldowns.rs`, `bundle.rs`, `item_nbt.rs`. Re-exported as `horizon_client::items`... |
| `horizon-command` | Command parsing (Brigadier): the vanilla tree, argument types, selectors, suggestions, the tree's network form. Re-exported as `horizon_client::command` |
| `horizon-client-core` | The client lib's bottom layer: `text.rs`, `lang.rs`, `snbt.rs`, `nbt_path.rs`, `component.rs`, `scoreboard.rs`, `bossbar.rs`, `gamerules.rs`, `block_entity_nbt.rs`, registry/particle/sound tables. Re-exported under the old paths (`horizon_client::text`...) |
| `horizon-client-input` | Key bindings and key names, toggle keys, mouse options, controller settings and glyphs, the clipboard, window options (`keybinds.rs`, `gamepad.rs`, `mouse.rs`, `window.rs`...). Re-exported under the old paths |
| `horizon-client-render` | The client's world data, rendering and screens (chunks, meshing, light, streaming, `gfx`, HUD, menus, settings), modules that horizon-client re-exports under the same paths. Nothing in it reaches gameplay |
| `horizon-world` | Vanilla world data with no gameplay loop: NBT, regions, chunks, `datafix/`, `worldgen/`, `level/` (server-side block behaviour, ticks, redstone), entity physics, items, effects, food, damage |
| `horizon-core` | Simulation primitives: Java `Random`s, `jmath` (HotSpot-exact `Math`), positions, noise, density, paletted storage, `version.rs` (the pinned version) |
| `horizon-protocol` | Network protocol 777: codecs, framing, encryption, `packets/{handshake,status,login,configuration,play}.rs`, `entity_data.rs` |
| `horizon-assets` | Resource decoding: block-state and item models, baking, atlases |
| `horizon-prepare` | Builds `.local/horizon-client/26.3` from the player's own Minecraft install (runs the Java exporters) |
| `horizon-server` | `horizon-server`: `server.toml`, CLI, supervisor of the headless game ([docs/hosting.md](docs/hosting.md)) |
| `horizon-server-files` | `server.properties`, eula, ops/whitelist/bans, usercache |
| `horizon-anticheat` | The dedicated server's anticheat, off by default: `[anticheat]` settings, violation levels, the checks' rules; the hub runs them (`lan/anticheat.rs`) ([docs/anticheat.md](docs/anticheat.md)) |
| `horizon-cosmetics` | Cosmetic items Horizon clients draw on players (off by default): the item format, bones and animations ([docs/cosmetics.md](docs/cosmetics.md)) |
| `horizon-addresses` | Every built-in address of an outside service vanilla doesn't contact (Horizon layer; [docs/layers.md](docs/layers.md#built-in-addresses)) |
| `horizon-friends`, `horizon-relay` | Play with Friends: invite codes, encrypted tunnel, the relay ([docs/play-with-friends.md](docs/play-with-friends.md)) |
| `horizon-mods`, `cargo-horizon-mod` | Content mods (`.hmod`: manifest, content fence, no code), the registry index and `.hpack`, and `cargo horizon-mod` (`new`, `build`, `run`, `check`, `registry`) ([docs/mods.md](docs/mods.md)). Code mods are native, built by the launcher ([docs/native-mods.md](docs/native-mods.md)) |
| `horizon-port-pack` | Porting Java modpacks ([docs/modpacks.md](docs/modpacks.md)) |
| `horizon-optifine` | OptiFine-format pack files below every feature that reads them: `.properties`, the name/range/block matchers, paths and `optifine/` folders, and custom entity models (`cem/`: `.jem`/`.jpm` parsing and baking) |
| `horizon-polytone` | Polytone-format pack files (`assets/*/polytone/`), read clean-room from the public wiki: targets, biome and dimension modifiers and their environment attribute layers (#11486) |
| `horizon-shader-api` | Horizon's own WGSL shader packs ([docs/api/shaders.md](docs/api/shaders.md)): `horizon-shaders.toml`, the `.hshader` package; shared by the game, the launcher and the registry |
| `horizon-shaders` | Shader packs (the `shaders/` folder format), rebuilt clean-room from a written spec of the format; no wgpu |
| `horizon-theme` | Design tokens and fonts for the launcher, the game's own menus and the website |
| `horizon-perf-format` | The `.hzperf` capture contract ([docs/perf-platform.md](docs/perf-platform.md) §3): `proto/horizon/perf/v1/capture.proto`, its generated types, the gzip writer and its thread. No Horizon dependencies; nothing in the game reads a capture |
| `horizon-telemetry`, `horizon-telemetry-server`, `horizon-bench` | Opt-in data collection, its ingest server, the benchmark report format |
| `horizon-system` | The machine the game runs on (OS, CPU, memory, drive, power, GPU maker and memory), for the crash report, the renderer's GPU info and telemetry |
| `horizon-platform` | The platform's public API: what the Horizon layer may use of the engine ([docs/layers.md](docs/layers.md)); re-exports `horizon-system` today |
| `horizon-hooks`, `horizon-game`, `horizon-official`, `horizon-heartbeat` | The layer seam: the engine's hooks for crates above it, the game an executable starts, the official executable (shipped as `horizon-client`) that registers Horizon's in-game parts, the first of them ([docs/layers.md](docs/layers.md#how-a-horizon-crate-plugs-in)) |
| `horizon-dualsense`, `horizon-threads`, `horizon-release-sign`, `horizon-bots`, `xtask` | DualSense and Xbox One output, thread priorities, signed release checksums, load-test bots, repo tasks |
| `launcher/` | GPUI launcher: `main.rs` (UI), `backend.rs` (installs, updates), `cli.rs`, `mcp.rs`, `*_page.rs` |

Other top-level folders: `tools/` (harnesses and dev scripts), `tests/` (Python unit tests: `python3 -m
unittest discover -s tests`), `mods/horizon/src/test/java/dev/horizonlab/parity/` (Java oracles),
`docs/`, `.github/workflows/`. The website (playhorizon.dev) is the private horizon-rust/horizon-site: its build
reads `README.md`, `launcher/links.json`, `docs/api/`, `roadmap/roadmap.json` and `crates/horizon-theme/tokens.css` from main.

## Write tests with signal

- Test observable behavior, not words in docs, comments or source. Keep checks for machine-consumed contracts.
- Use independent expected values or a vanilla oracle; do not derive the answer from the code under test.
- Test outcomes, not only calls to stubs. Use mocks to isolate behavior, not as the behavior itself.
- Skip tests of trivial constants, derives and getters, or behavior already covered by another test.
- One test per changed behaviour is enough. A refactor or a move that changes no behaviour gets no new test.
- In review, ask for a missing test only when it would catch a real regression, and flag a new test that breaks
  these rules.
- Avoid pinning implementation details; a harmless refactor should not break a behavior test.
- Make slow tests faster without removing cases or weakening their coverage.
- Never delete parity harnesses or tests that pin vanilla behaviour: they are the spec.

## Build and test fast

```sh
python3 tools/pr_ready.py                  # the self-check: fmt, clippy -D warnings, touched crates' tests; paste its block in the PR
python3 tools/pr_ready.py --harnesses      # also the affected harnesses (by default only when you change a harness, oracle, probe or the runner, as in CI)
cargo test -p horizon-client -p horizon-client-lib --lib --bins --tests   # skips the examples (pr_ready does this); `--doc` separately
cargo test -p horizon-client-lib --lib mod_content::tests:: -- --ignored   # the slow content-root tests, #[ignore]d; pr_ready runs them when mod content, assets, blocks or items change
cargo build --profile fast -p horizon-game          # optimized, quick to link: use it to play or run lan_e2e
python3 tools/parity.py --changed origin/main       # the harnesses your branch can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt --list   # which harnesses cover these files, without running them
python3 tools/parity.py mob_ai sculk       # named harnesses
python3 tools/lan_e2e.py --check chest     # two real games over LAN (server-thread host)
python3 tools/agent_control_e2e.py         # horizon-ctl / MCP end to end
python3 tools/lang_coverage.py --summary   # one line per language: how many keys it carries and what it misses
python3 tools/lang_coverage.py --summary --show-keys   # the same, then the keys behind the counts
```

- To compile and test on hosted runners instead of this PC, start a try run on your pushed branch:
  `gh workflow run checks --ref <branch>`, `-f windows=true` for a Windows x64 build to download
  ([docs/ci-runners.md](docs/ci-runners.md#try-runs)).
- Harnesses and the game need the vanilla data in `.local/` (`minecraft/`, `horizon-client/26.3`, `datagen/`,
  `tools/`): see [docs/parity-workflow.md](docs/parity-workflow.md). Without it, pr_ready lists the
  harnesses for CI.
- A harness = `tools/test_<name>_parity.py` + a Java oracle (`mods/horizon/src/test/java/dev/horizonlab/parity/Vanilla*Oracle.java`)
  + a Rust probe: `crates/<crate>/examples/<name>_{probe,fixture}.rs` (mostly horizon-world and horizon-client),
  or the game binary with a fixture env var (`game/interface/dispense_fixture.rs`, `game/blocks/sculk_fixture.rs`,
  `game/administration/command_harness.rs`, `game/network/net_harness.rs`). Copy a nearby harness for a new one.
- Developing a native mod? Run `horizon-launcher settings native-profile dev` once. `dev` builds the game crate
  unoptimized: a mod edit rebuilds about 6× faster than with `performance`, while game logic runs up to about twice as
  slow. Use it while developing, not for playing; `performance` stays the default for players and releases
  ([docs/native-mods.md](docs/native-mods.md)).
- Probes build into `.local/parity-target/`; harnesses find binaries via `lab.cargo_target()`, never `target/`.
- Harness selection uses Cargo dep-info and reachable-code narrowing (`tools/parity_reach.py`).
  `horizon-client-lib` builds the control tool and client probes without gameplay; their dep-info
  excludes `game.rs` and `game/`, so gameplay edits do not rebuild or select those probes.
  Source paths remain in `crates/horizon-client/`. Measure actual harness selection on refreshed
  records with `tools/parity.py --changed-files paths.txt --list`; historical counts are not a guarantee.
- Rust integration tests: `crates/horizon-client/tests/` (`lan_bot.rs`, `generation_saves.rs`, `streaming_meshes.rs`).
- Linux needs `libasound2-dev libudev-dev`; `--no-default-features` drops audio, controllers, `discord`, `friends` and `heartbeat` (the server build).
- Run and drive the game (starts the `horizon-client` beside `horizon-ctl`; other arguments go to the game):
  `target/fast/horizon-ctl --socket $PWD/ctl.sock launch --world W --creative --offscreen --wait --resources $PWD/.local/horizon-client/26.3`,
  then `horizon-ctl --socket … state|keys|turn|command|screenshot|quit`
  ([docs/horizon-ctl-commands.md](docs/horizon-ctl-commands.md)).

## Vanilla reference

- Start parity work from the port ledger, not grep: `python3 tools/port_ledger.py issue N` (or `where
  Class.method`, `what file:line`), and `check --base origin/main` before the PR. How:
  [.claude/skills/vanilla-parity/SKILL.md](.claude/skills/vanilla-parity/SKILL.md), [docs/port-ledger.md](docs/port-ledger.md).
- Decompiled sources: `.local/minecraft/26.3/sources/{client,server}/` (`python3 tools/lab.py decompile`).
- Bytecode queries (callers through lambdas, every overload):
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`
  ([docs/parity-workflow.md](docs/parity-workflow.md#reading-vanilla)).
- A harness runs the oracle against the real 26.3 jars and the Rust probe on the same inputs, then compares
  their JSON. Oracle runs are cached by `tools/parity_shim.py`; `--fresh` reruns them.
- Mirror vanilla's `Math` calls with `horizon_core::jmath`, and vanilla's RNG with `horizon_core::random`:
  random draws must happen in vanilla's order. Performance work may break that order where no player can rely
  on it; the pull request says so ([CONTRIBUTING.md](CONTRIBUTING.md#when-it-doesnt-have-to-be-11)).

## horizon-client: where things live

`L:` = lib (`crates/horizon-client/src/`, or `crates/horizon-client-render/src/` for the modules that crate holds, under
the same `horizon_client::<module>` paths), `G:` = gameplay (`crates/horizon-client/src/game/`), also in the lib in normal builds.
Gameplay keeps its `horizon_client::<module>` imports through a self-crate alias. The executable
contains only the allocator, mod API marker and a call to `entry::main`. For unit tests, the lib
excludes `entry` and `game`, and the executable test target owns `game` so its process-wide test
state stays separate from the library tests. This compiles gameplay twice for executable tests.

| Subsystem | Files |
|---|---|
| Game loop, tick, frame | `game.rs`: `fn run`, `fn tick`, `fn tick_world`, `fn render`; G: `runtime/window_handler.rs` (`impl ApplicationHandler for Game`: `window_event`, `about_to_wait`), `runtime/launch_options.rs` (the command line); L: `clock.rs`, `spans.rs`; `horizon-client-input/src/window.rs` |
| Rendering | L: `gfx.rs` (`Renderer::new`, `Renderer::render`), `gfx/` (cull, occlusion, translucency, upload, timing, lod, weather, stars, end_sky), `*.wgsl`, `particles.rs`, `block_entity_render.rs`, `item_render.rs`, `fog.rs`, `clouds.rs`, `camera.rs`, `first_person.rs`; G: `interface/render_tick.rs`, `interface/frame_geometry.rs` (cracks, sun and moon, first-person hand, rain and snow), `interface/item_draw.rs`, `interface/screen_backgrounds.rs` (panorama, menu blur, border wall) |
| Chunks, streaming, light | L: `streaming.rs` (worker pool: generate, light, mesh), `world.rs`, `chunk.rs`, `chunk_map.rs`, `light.rs`, `mesher.rs` + `mesher/`, `vanillagen.rs` (binds horizon-world's generator), `lod.rs` + `lod/`, `server_terrain.rs` (a server's kept chunks), `ticket_levels.rs`; G: `world/world_loading.rs`, `world/world_entry.rs`, `world/postprocess.rs`, `world/world_update.rs` (`fn update_world`, far terrain) |
| World generation | `crates/horizon-world/src/worldgen/` (`generator*.rs`, `feature_graph.rs`, `structure/`, `carver/`) |
| Block behaviour | `crates/horizon-world/src/level/` (`mod.rs`, `tick.rs`, `hooks.rs`, `redstone.rs`, `place.rs`, `interact.rs`); G: `blocks/block_ticks.rs` (`fn tick_blocks`); game.rs `fn process_level_events` |
| Mobs | L: `mobs.rs` (types), `mobs/` (`ai.rs`, `goals.rs`, `brain.rs`, `*_brain.rs`, `nav.rs`, `path.rs`, `species.rs`, `spawning.rs`, `render.rs`, `anim.rs`); G: `creatures.rs` (ticking, spawning, mob events), one file per mob family (`mobs/dragon.rs`, `mobs/wither.rs`, `mobs/villagers.rs`, `mobs/piglins.rs`, `mobs/golems.rs`...), `mobs/natural_spawning.rs` |
| Other entities | L: `entities.rs`, `projectiles.rs`, `arrows.rs`; G: `items/item_entities.rs` (dropped items, experience orbs), `boats.rs`, `entities/minecarts.rs`, `mobs/mounts.rs`, `entities/armor_stands.rs`, `entities/paintings.rs`, `frames.rs`, `entities/fireworks.rs`, `entities/tnt.rs`, `entities/explosion.rs` |
| Player actions | game.rs (`fn tick_mining`, `fn break_block`); G: `items/use_hand.rs` (`fn use_hand`), `items/item_actions.rs` (`fn use_item`), `entities/melee.rs` (`fn attack`), `items/hand.rs`, `items/combat.rs`, `items/consume.rs`, `items/using_item.rs` (an item in use, tick by tick), `items/potions.rs`, `items/shield.rs`, `items/crossbow.rs`, `items/trident.rs`, `items/fishing.rs`; L: `motion.rs` + `motion/` (movement), `collisions.rs`; crate `horizon-items`: `items.rs`, `item_data.rs`, `enchant.rs`, `blocks.rs` |
| Inventory and containers | L: `menu.rs` (`AbstractContainerMenu.clicked`), `container_screen.rs`, `containers.rs`, `creative.rs`, `crafting.rs`, `recipe_book*.rs`, `stations.rs`, `merchant.rs`; G: `interface/menu_clicks.rs` (`fn menu_ops`, `fn click_inventory`), `blocks/stations.rs`, `items/trading.rs`, `items/recipe_book.rs`, `machines.rs`, `interface/inventory_input.rs` (keys and clicks over a container), `interface/inventory_slots.rs` (slot layout and drawing) |
| Commands | parse: crate `horizon-command` (`args.rs`, `selector.rs`, `tree_*.rs`, `vanilla.rs`); run: G: `commands.rs` (dispatch) + `commands/*.rs` (`admin`, `execute`, `items`, `players`, `world_cmds`...); L: `scoreboard_cmd.rs`, `data_cmd.rs`; `horizon-client-core`: `nbt_path.rs`, `scoreboard.rs`; G: `administration/functions.rs`, `administration/command_blocks.rs` |
| UI screens | `Screen` enum in game.rs; L: `title.rs` + `title/`, `options_ui.rs`, `multiplayer.rs`, `pack_screen.rs`, `settings_shell.rs` (Horizon's settings screens on it: `extras_ui.rs`, `graphics_ui.rs`, `controller_ui.rs`, `shader_ui.rs`; a page list too long for its column collapses into one row), `screen_draw.rs`, `ui.rs` (widgets), `hud.rs`, `hud_layout.rs`, `hud_overlays.rs`, `hud_kit/` (Horizon overlays' `Room` and `Plate`, the collapsed page strip `tabs` the shell's page list uses, [docs/ui-kit.md](docs/ui-kit.md)), `pad_glyph.rs` (controller glyphs, by what the button does), `chat.rs`, `tab_list.rs`; G: `interface/startup_menu.rs`, `runtime/loading_screen.rs` (the loading screen, the pre-game `App`), `runtime/resource_loader.rs` (what a start loads, `run_loading`), `interface/screens.rs` (pause, death), `interface/options.rs`, `network/lan_screen.rs`, `interface/chat.rs`, `interface/sign_edit.rs`, `interface/books.rs`, `interface/creative_screen.rs` |
| Multiplayer client | L: `net.rs` (network thread); G: `network/remote/mod.rs` (each `Game` method's mod point, forwarding to `<name>_body` in its feature file), `network/remote/apply_packet.rs` (`fn apply_packet_body`), `network/remote/tick.rs` (`fn tick_remote_body`), `network/remote/` (mob data, objects, projectiles, vehicles, stations) |
| LAN host / server side | L: `lan/` (`conn.rs` handshake + login threads, `hub.rs` + `hub/` play state, `chunks.rs`, `entity.rs`, `tracking.rs`, `guest.rs`, `playerdata.rs`); G: `lan.rs` (`LanHost`), `lan_actions.rs` (`fn as_guest`, `fn tick_guests`), `lan_*.rs` per feature, `network/lan_entities.rs` (tracked entities) |
| Integrated server | G: `world/integrated.rs` (the server thread), `world/levels.rs` (one level per dimension, `fn tick_server`), `world/server_travel.rs` |
| Dedicated server | G: `dedicated.rs`, `dedicated/status.rs`; crates `horizon-server-files`, `horizon-server`, `horizon-anticheat` (L: `lan/anticheat.rs`) |
| Save and load | L: `save.rs` (level.dat, regions, playerdata), `entity_nbt.rs`, `options_file.rs`; `horizon-client-core/src/block_entity_nbt.rs`; G: `world/saving.rs`, `world/persist.rs` (entities), `world/world_session.rs` (opening and closing a world); `crates/horizon-world/src/{region.rs,nbt.rs,datafix/}` |
| Input | game.rs `fn window_event`; G: `interface/play_keys.rs` (`fn key_pressed`), `interface/play_input.rs` (`fn play_mouse`, toggled keys, the hotbar wheel); crate `horizon-client-input` (`keybinds.rs`, `keymapping.rs`, `key_names.rs`, `mouse.rs`, `input.rs`, `interaction_input.rs`, `gamepad.rs`, `clipboard.rs`); L: `osk.rs`; G: `pad.rs`, `interface/pad_menu.rs`, `interface/pad_craft.rs`, `interface/pointer_capture.rs` (mouse look, pointer grab) |
| Audio | L: `audio.rs` (mixer), `sound_engine.rs`, `sound_events.rs`, `music.rs`, `ambience.rs`; G: `interface/music.rs`, `interface/subtitle_hud.rs` |
| Resource packs | L: `packs.rs` + `packs/`, `pack_screen.rs`, `atlas_sources.rs`, `sprites.rs`, `mipmap.rs`, `model_library.rs`, `font.rs`, `mobs/cem.rs` + `mobs/cem_anim.rs` (OptiFine/EMF entity models and their animations); `horizon-client-core/src/lang.rs`; crates `horizon-assets`, `horizon-optifine`; G: `packs.rs` |
| Agent control | L: `control.rs` (socket), `bin/horizon-ctl/`; G: `runtime/control.rs`, `agent.rs`, `events.rs`, `runtime/ui_control.rs` ([docs/agent-control.md](docs/agent-control.md)) |
| Mods | crate `horizon-mods`; G: `runtime/hot_paths/mods.rs`; L: `mod_content.rs`, `mod_list.rs` |
| Horizon extras, settings | L: `extras.rs`, `graphics.rs`, `settings.rs`, `minimap.rs`; G: `interface/extras.rs` + `extras/` (each extra's glue: `interface/extras/mouse_tweaks.rs`, `interface/extras/sorting.rs`, `entities/pvp.rs`, `interface/extras/reconnect.rs`), `interface/options.rs` |

## How `Game` is organised

- `struct Game` (game.rs) holds everything. Its methods are spread over many files: game.rs keeps
  the core (tick, render, input, inventory, world open/close) and each `game/<feature>.rs` adds an
  `impl Game<'_>` block for one feature, with its own state types. Fields stay private to the gameplay module and its descendants. Gameplay tests run in the gameplay library, separate from reusable-library tests.
- Find a feature's code: `grep -rn "fn tick_<thing>\|fn <verb>_<thing>" crates/horizon-client/src/game*`;
  the file is usually `game/<thing>s.rs`. Who calls it: grep the name in game.rs's `fn tick` / `fn tick_world`.
- Fields that say which mode runs: `remote` (this window is a client of a server: multiplayer, LAN guest, or
  single player on its server thread), `lan` (`LanHost`: this world is open to the network), `integrated`
  (the window's own server thread), `serving` (this `Game` *is* that server thread), `options.dedicated`.
  `fn tick` returns `tick_remote()` when `remote` is set; a server ticks its levels (`fn tick_world`).
- A server's players run through the shared gameplay code: `as_guest` swaps the guest's
  state (position, inventory, mode, mining, item use) into `Game` for one call (`game/lan_actions.rs`), and
  `levels.rs` swaps whole dimensions in the same way. Changes reach clients through the block log, level
  events and the entity tracker (`lan/tracking.rs`, `game/network/lan_entities.rs`).
- Shared read-only data: `Resources` (`res`: blocks, items, recipes, mob types, biomes) and `PackResources`
  (`pack`), both in game.rs, their methods in `game/runtime/resource_tables.rs`.

## Single player runs on a server thread

`game/world/integrated.rs` runs a second `Game` (`serving`) on a "Server thread" with one level per dimension;
the window joins it as a client through memory (`remote`), exactly as a LAN guest or a dedicated server's
player does (#1514). A window's `Game` never ticks a world itself: the old in-frame path is gone
(`--in-frame` and `--no-server-thread` only print a notice; `HORIZON_SERVER_THREAD` is ignored).
Open `[server-thread]` issues list the gaps left.

Single player, LAN guests and dedicated servers all run the server code, so a fix there serves all three.
When you change how the server and its clients talk (packets, tracking, guests, the server thread itself), check
it there. Other gameplay fixes are covered by their own harness and CI's parity job:

```sh
cargo run --profile fast -p horizon-game -- --resources .local/horizon-client/26.3 --world T                 # server thread
python3 tools/lan_e2e.py --check <name>                  # host on its server thread, guest is a server client
python3 tools/parity.py dedicated_server server_entities server_player_damage
```

## Docs worth knowing

[docs/parity-workflow.md](docs/parity-workflow.md) (runner, gate, shards, scopes) ·
[docs/native-client.md](docs/native-client.md) (options, controls, horizon-ctl) ·
[docs/server-levels.md](docs/server-levels.md) (levels per dimension) ·
[docs/rust-parity.md](docs/rust-parity.md) (what is ported) · [docs/known-issues.md](docs/known-issues.md) ·
[docs/release-flow.md](docs/release-flow.md) · [docs/README.md](docs/README.md) (index).
