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
- **Mark what an agent posts.** A PR body starts with a line naming the person whose agent opened it and
  the tool: `> 🤖 Opened by <login>'s AI agent (<tool, model>), not by <login> personally.`, where `<login>`
  is the GitHub account the PR is opened from (`gh api user --jq .login`). Issues say "Filed by", reviews
  "Review by". Every comment starts with `🤖 Agent:`. Write only as the person you work for: the lines on
  this page that name Ludovic are for his own agents (account `ludovic111`), never for anyone else's.
- **Attribution.** Commits end with a `Co-Authored-By:` line for the agent that wrote them, and PR bodies
  with that agent's own "Generated with" line. Example, Ludovic's agents (Claude Code on `ludovic111`):
  first line `> 🤖 Opened by Ludovic's AI agent (Claude), not by Ludovic himself.`, commits
  `Co-Authored-By: Claude <model> <noreply@anthropic.com>`, last line
  `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- Everything players run is Rust. Python/Java only for dev tools and harness drivers in `tools/`.
- Horizon-only features follow [docs/design-system.md](docs/design-system.md) and are off by default where
  they would change the vanilla look.
- One concern per PR, at whatever size it needs; no line limit on the fork. Labels: `feature` (players get something new), `skip-release` (they see no
  change), none for a fix. Agents never add `release-now`: the maintainer decides it. Merges reach players through the
  nightly, published by the daily run when every leg is green ([docs/release-flow.md](docs/release-flow.md#daily-run)).
- **CI tiers** ([CONTRIBUTING.md](CONTRIBUTING.md#ci-tiers)): every PR push and main push runs `checks`
  (`verify`: format, lint and unit tests only; `launcher`); the ignored-test shards, the wasm
  fixture, `fn-replace-check`, the SDK build and its linkage probe run once a day (`daily.yml`). A new test that needs prepared
  resources, boots the game, starts a child process, uses the network or a compiled wasm fixture, or takes 2 s or
  more is `#[ignore]`d with a row in `tools/ci/ignored-tests/`; say in the PR when you could not run it. A daily red
  becomes an issue listing the suspect PRs, fixed by an ordinary PR. Existing tests still in `verify` that need
  resources, a child process or the network are being moved by the next change of #4022. Builds: snapshot (a commit of main, gate green; published as
  `snapshot-<date>-<short>`, which no launcher offers), nightly (published by a fully green daily run), release (cut from a nightly).
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

## Mod API on this fork: the clean break

This fork (arismoko/horizon) is where the native mod API is built: hooks, the `Host`, handles, Parts,
attachments and mod loading. Tracker: #26 (design), #44 (milestones). The mod API is taking **one clean
break**, and every change to it follows these rules. Reviews block a PR that breaks any of them.

- **No compatibility paths.** No aliases, shims, deprecated names, migration or fallback code, version
  gates, or "kept for old mods" branches. Old mods rebuild; the compiler says what moved. Removing an old
  path updates every caller, the catalog, docs and tests in the same change.
- **One hook per moment.** A hook with a NeoForge 26.3 counterpart takes its name, side and full contract.
  Two hooks at the same vanilla call site (including `_v1`/`_v2` pairs) merge into that one, and the old
  one is deleted, not kept beside it. A veto before an action and a notice after it are separate moments
  when vanilla runs code between them. A hook whose vanilla call site is gone is removed from the catalog.
- **Every hook gets a `Host`.** Hooks that fire inside client-lib split into decide, fire, apply: client-lib
  decides, the game fires the hook with a `Host`, then applies. The listener's writes and the apply run in
  one transaction, and the apply re-checks the decision's preconditions and refuses (never recomputes) if
  they changed.
- **`Host` shape.** A small shared `Host` (handle constructors, registries, settings, `transaction`), with
  `ServerHost: Host` and `ClientHost: Host` generated from each catalog entry's `side`, so a wrong-side
  call doesn't compile; a hook that fires on both sides gets a typed `HostSide`. Per-object methods live on
  handles (reads take `&dyn Host`, writes `&mut dyn ServerHost`), and the ID-based helper a handle method
  covers is removed when it lands. Drawing goes through a `DrawContext` given only to render and screen
  hooks. Items, fluids and energy go through Parts under transactions. Commands, players, messages and
  scheduling are small service traits reached from `Host`; attachments live on handles. Don't add a
  method to `Host` that has a natural receiver.
- **One dependency model, two build targets.** Native and wasm mods share exactly the same dependency
  model; the only difference is the build target. `[dependencies]` in `horizon-mod.toml` (required and
  optional, semver on `horizon-mod.toml` versions, dependency-ordered dispatch, a mod publishing functions
  and statics its dependents call) means the same for both: native links through Cargo, wasm through the
  runtime's linker, and one copy of a library is shared by its dependents. No wasm-only packaging unit or
  multi-mod bundle, and no rule that holds for one target only. A change to the dependency model lands
  for both targets in the same PR.
- **Versions are the exact build for native mods.** A native mod is built against one game build and the
  loader checks that build id; a wasm mod links by interface instead. `.hmod` is the one package format.
- **DRY.** One implementation per behaviour: reuse the existing helper, write path or generator rule, and
  let the catalog and generator own repetition instead of hand-written match arms.
- **SOLID.** Each module, trait and handle has one job; traits stay small and split by side and concern;
  new behaviour comes in through the catalog, handles and parts; mods depend on `mod_api`, never on game
  internals, and lower crates never reach up into `Game`.
- **Neat.** Names, layout and docs match the code around them. No dead code, leftover TODO paths, or
  anything a later PR must tidy up.
- **Fast.** No allocation, string building or scan over every mod on hot paths (tick, per-entity and
  per-block hooks). Dispatch only to the mods and parts that subscribe, resolve keys once at load, and a
  PR that touches a hot path states its tick-time effect.
- **Differences from upstream are marked.** A change that renames or removes something horizon-rust/horizon
  also has says "Diverges from upstream" in its PR body. Write upstream refs in full
  (`horizon-rust/horizon#N`; a bare `#N` is this fork) and never put Fixes, Closes or Resolves before one.

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
  harness where vanilla has nothing to compare (Horizon's own features, performance work).
- **A red check isn't always yours.** When `verify` fails in a test your change can't reach, say so in the pull
  request and move on; don't hunt it.
- **Unsure about a rule? Finish the work.** Take the reading that lets you finish, and say in the pull request
  which one you took. Don't stop, and don't change direction halfway.

## Crates

Game workspace (root `Cargo.toml`); `launcher/` and `crates/horizon-telemetry-server` are separate workspaces.

| Crate | Owns |
|---|---|
| `horizon-client` | Gameplay library (`src/lib.rs`: `game.rs` + `game/`, `entry.rs`) and thin executable (`src/main.rs`). Also the dedicated server (`--dedicated`). Re-exports `horizon-client-lib` for existing mod paths. |
| `horizon-client-lib` | Reusable client systems, `horizon-ctl`, probes and their tests. Its manifest points to the existing sources under `crates/horizon-client/`; the library root is `src/client_lib.rs`. It never depends on gameplay. |
| `horizon-items` | Blocks and items below gameplay: `blocks.rs` (block facts), `items.rs`, `item_data.rs` (components), `enchant.rs`, `cooldowns.rs`, `bundle.rs`, `item_nbt.rs`. Re-exported as `horizon_client::items`... |
| `horizon-command` | Command parsing (Brigadier): the vanilla tree, argument types, selectors, suggestions, the tree's network form. Re-exported as `horizon_client::command` |
| `horizon-client-core` | The client lib's bottom layer: `text.rs`, `lang.rs`, `snbt.rs`, `nbt_path.rs`, `component.rs`, `scoreboard.rs`, `bossbar.rs`, `gamerules.rs`, `block_entity_nbt.rs`, registry/particle/sound tables. Re-exported under the old paths (`horizon_client::text`...) |
| `horizon-client-input` | Key bindings and key names, toggle keys, mouse options, controller settings and glyphs, the clipboard, window options (`keybinds.rs`, `gamepad.rs`, `mouse.rs`, `window.rs`...). Re-exported under the old paths |
| `horizon-client-render` | The client's world data, rendering and screens (chunks, meshing, light, streaming, `gfx`, HUD, menus, settings): ~80 modules that horizon-client re-exports under the same paths. Nothing in it reaches gameplay |
| `horizon-mod-runtime` | Engine-independent mod attachment storage, query index and generic registry slot tables; depends only on `horizon-core` |
| `horizon-world` | Vanilla world data with no gameplay loop: NBT, regions, chunks, `datafix/`, `worldgen/`, `level/` (server-side block behaviour, ticks, redstone), entity physics, items, effects, food, damage |
| `horizon-core` | Simulation primitives: Java `Random`s, `jmath` (HotSpot-exact `Math`), positions, noise, density, paletted storage, `version.rs` (the pinned version) |
| `horizon-protocol` | Network protocol 777: codecs, framing, encryption, `packets/{handshake,status,login,configuration,play}.rs`, `entity_data.rs` |
| `horizon-assets` | Resource decoding: block-state and item models, baking, atlases |
| `horizon-prepare` | Builds `.local/horizon-client/26.3` from the player's own Minecraft install (runs the Java exporters) |
| `horizon-server` | `horizon-server`: `server.toml`, CLI, supervisor of the headless game ([docs/hosting.md](docs/hosting.md)) |
| `horizon-server-files` | `server.properties`, eula, ops/whitelist/bans, usercache |
| `horizon-java-properties` | `java.util.Properties` load/store and the JDK date text, standard library only: shared by `horizon-server-files` and the wasm runtime |
| `horizon-anticheat` | The dedicated server's anticheat, off by default: `[anticheat]` settings, violation levels, the checks' rules; the hub runs them (`lan/anticheat.rs`) ([docs/anticheat.md](docs/anticheat.md)) |
| `horizon-friends`, `horizon-relay` | Play with Friends: invite codes, encrypted tunnel, the relay ([docs/play-with-friends.md](docs/play-with-friends.md)) |
| `horizon-mod-package`, `cargo-horizon-mod` | Content mods (`.hmod`: manifest, content fence, no code), the registry index and `.hpack`, and `cargo horizon-mod` (`new`, `build`, `run`, `check`, `registry`) ([docs/modding/](docs/modding/src/introduction.md)). Code mods are native, built by the launcher ([docs/modding/src/how-to/native-mods.md](docs/modding/src/how-to/native-mods.md)) |
| `horizon-port-pack` | Porting Java modpacks ([docs/modpacks.md](docs/modpacks.md)) |
| `horizon-optifine` | OptiFine-format pack files below every feature that reads them: `.properties`, the name/range/block matchers, paths and `optifine/` folders, and custom entity models (`cem/`: `.jem`/`.jpm` parsing and baking) |
| `horizon-polytone` | Polytone-format pack files (`assets/*/polytone/`), read clean-room from the public wiki: targets, biome and dimension modifiers and their environment attribute layers (#11486) |
| `horizon-shader-api` | Horizon's own WGSL shader packs ([docs/api/shaders.md](docs/api/shaders.md)): `horizon-shaders.toml`, the `.hshader` package; shared by the game, the launcher and the registry |
| `horizon-shaders` | Shader packs (the `shaders/` folder format), rebuilt clean-room from a written spec of the format; no wgpu |
| `horizon-theme` | Design tokens and fonts for the launcher, the game's own menus and the website |
| `horizon-perf-format` | The `.hzperf` capture contract ([docs/perf-platform.md](docs/perf-platform.md) §3): `proto/horizon/perf/v1/capture.proto`, its generated types, the gzip writer and its thread. No Horizon dependencies; nothing in the game reads a capture |
| `horizon-telemetry`, `horizon-telemetry-server`, `horizon-bench` | Opt-in data collection, its ingest server, the benchmark report format |
| `horizon-cosmetics` | Cosmetic items Horizon clients draw on players (off by default): the item format, bones and animations ([docs/cosmetics.md](docs/cosmetics.md)) |
| `horizon-addresses` | Every built-in address of an outside service vanilla doesn't contact (Horizon layer; [docs/layers.md](docs/layers.md#built-in-addresses)) |
| `horizon-system` | The machine the game runs on (OS, CPU, memory, drive, power, GPU maker and memory), for the crash report, the renderer's GPU info and telemetry |
| `horizon-platform` | The platform's public API: what the Horizon layer may use of the engine ([docs/layers.md](docs/layers.md)); re-exports `horizon-system` today |
| `horizon-hooks`, `horizon-game`, `horizon-official`, `horizon-heartbeat` | The layer seam: the engine's hooks for crates above it, the game an executable starts, the official executable (shipped as `horizon-client`) that registers Horizon's in-game parts, the first of them ([docs/layers.md](docs/layers.md#how-a-horizon-crate-plugs-in)) |
| `horizon-dualsense`, `horizon-threads`, `horizon-release-sign`, `horizon-bots`, `xtask` | DualSense output, thread priorities, signed release checksums, load-test bots, repo tasks |
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
- Avoid pinning implementation details; a harmless refactor should not break a behavior test.
- Make slow tests faster without removing cases or weakening their coverage.
- Never delete parity harnesses or tests that pin vanilla behaviour: they are the spec.

## Build and test fast

```sh
python3 tools/pr_ready.py                  # the self-check: fmt, clippy -D warnings, touched crates' tests, harnesses; paste its block in the PR
python3 tools/pr_ready.py --quick          # same without harnesses
cargo test -p horizon-client -p horizon-game -p horizon-client-lib --lib --bins --tests   # skips the examples (pr_ready does this); `--doc` separately
cargo test -p horizon-client-lib --lib mod_content::tests:: -- --ignored   # the slow content-root tests, #[ignore]d; pr_ready runs them when mod content, assets, blocks or items change
cargo build --profile fast -p horizon-game        # optimized, quick to link: use it to play or run lan_e2e
python3 tools/parity.py --changed origin/main       # the harnesses your branch can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt --list   # which harnesses cover these files, without running them
python3 tools/parity.py mob_ai sculk       # named harnesses
python3 tools/lan_e2e.py --check chest     # two real games over LAN (server-thread host)
python3 tools/agent_control_e2e.py         # horizon-ctl / MCP end to end
python3 tools/lang_coverage.py --summary   # one line per language: how many keys it carries and what it misses
python3 tools/lang_coverage.py --summary --show-keys   # the same, then the keys behind the counts
```

- GPU tests share `gfx::test_gpu::lock()`: bind the guard before creating GPU objects so it outlives
  every resource, queue, device and instance, including GPU-owning workers (join them before unlocking).
  Helpers returning GPU objects leave the guard with their caller; helpers returning only CPU data may
  own it themselves, without another outer guard. CPU-only tests need no guard. Run renderer coverage
  with `cargo test -p horizon-client-render --lib -- gfx:: --test-threads=12`; no global serial-test flag.

- For a player-facing change, drive the game once with `horizon-ctl` (launch, the action, a `state` or `screenshot`)
  and report the result in the PR's "Tried in game" line; never claim it when it was not run.
- Harnesses and the game need the vanilla data in `.local/` (`minecraft/`, `horizon-client/26.3`, `datagen/`,
  `tools/`): see [docs/parity-workflow.md](docs/parity-workflow.md). Without it, pr_ready lists the
  harnesses for CI.
- A harness = `tools/test_<name>_parity.py` (324 of them) + a Java oracle (`mods/horizon/src/test/java/dev/horizonlab/parity/Vanilla*Oracle.java`)
  + a Rust probe: `crates/<crate>/examples/<name>_{probe,fixture}.rs` (horizon-world 175, horizon-client 104),
  or the game binary with a fixture env var (`game/interface/dispense_fixture.rs`, `game/blocks/sculk_fixture.rs`,
  `game/administration/command_harness.rs`, `game/network/net_harness.rs`). Copy a nearby harness for a new one.
- Native mods compile against a pinned prebuilt SDK; `[[native.mixin]]` rows compile into a function-replacement patch library.
  Use `native-build --trust-native-code`; the launcher never rebuilds the game for a mod.
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
  ([docs/decompilation-status.md](docs/decompilation-status.md)).
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
| Game loop, tick, frame | `game.rs`: `fn run`, `impl ApplicationHandler for Game` (`window_event`, `about_to_wait`), `fn tick`, `fn tick_world`, `fn render`; L: `clock.rs`, `spans.rs`; `horizon-client-input/src/window.rs` |
| Rendering | L: `gfx.rs` (`Renderer::new`, `Renderer::render`), `gfx/` (cull, occlusion, translucency, upload, timing, lod, weather, stars, end_sky), `*.wgsl`, `particles.rs`, `block_entity_render.rs`, `item_render.rs`, `fog.rs`, `clouds.rs`, `camera.rs`, `first_person.rs`; G: `render_tick.rs`, `item_draw.rs`, `screen_backgrounds.rs` (panorama, menu blur, border wall) |
| Chunks, streaming, light | L: `streaming.rs` (worker pool: generate, light, mesh), `world.rs`, `chunk.rs`, `chunk_map.rs`, `light.rs`, `mesher.rs` + `mesher/`, `vanillagen.rs` (binds horizon-world's generator), `lod.rs` + `lod/`, `server_terrain.rs` (a server's kept chunks), `bluemap.rs` + `bluemap_detect.rs` (far terrain from its web map), `ticket_levels.rs`; G: `world_loading.rs`, `world_entry.rs`, `postprocess.rs`; game.rs `fn update_world` |
| World generation | `crates/horizon-world/src/worldgen/` (`generator*.rs`, `feature_graph.rs`, `structure/`, `carver/`) |
| Block behaviour | `crates/horizon-world/src/level/` (`mod.rs`, `tick.rs`, `hooks.rs`, `redstone.rs`, `place.rs`, `interact.rs`); game.rs `fn tick_blocks`, `fn process_level_events` |
| Mobs | L: `mobs.rs` (types), `mobs/` (`ai.rs`, `goals.rs`, `brain.rs`, `*_brain.rs`, `nav.rs`, `path.rs`, `species.rs`, `spawning.rs`, `render.rs`, `anim.rs`); G: `creatures.rs` (ticking, spawning, mob events), one file per mob family (`dragon.rs`, `wither.rs`, `villagers.rs`, `piglins.rs`, `golems.rs`...), `natural_spawning.rs` |
| Other entities | L: `entities.rs`, `projectiles.rs`, `arrows.rs`; G: `boats.rs`, `minecarts.rs`, `mounts.rs`, `armor_stands.rs`, `paintings.rs`, `frames.rs`, `fireworks.rs`, `tnt.rs`, `explosion.rs` |
| Player actions | game.rs (`fn use_hand`, `fn use_item`, `fn attack`, `fn tick_mining`, `fn break_block`); G: `hand.rs`, `combat.rs`, `consume.rs`, `using_item.rs` (an item in use, tick by tick), `potions.rs`, `shield.rs`, `crossbow.rs`, `trident.rs`, `fishing.rs`; L: `motion.rs` + `motion/` (movement), `collisions.rs`; crate `horizon-items`: `items.rs`, `item_data.rs`, `enchant.rs`, `blocks.rs` |
| Inventory and containers | L: `menu.rs` (`AbstractContainerMenu.clicked`), `container_screen.rs`, `containers.rs`, `creative.rs`, `crafting.rs`, `recipe_book*.rs`, `stations.rs`, `merchant.rs`; game.rs `fn menu_ops`, `fn click_inventory`; G: `stations.rs`, `trading.rs`, `recipe_book.rs`, `machines.rs` |
| Commands | parse: crate `horizon-command` (`args.rs`, `selector.rs`, `tree_*.rs`, `vanilla.rs`); run: G: `commands.rs` (dispatch) + `commands/*.rs` (`admin`, `execute`, `items`, `players`, `world_cmds`...); L: `scoreboard_cmd.rs`, `data_cmd.rs`; `horizon-client-core`: `nbt_path.rs`, `scoreboard.rs`; G: `functions.rs`, `command_blocks.rs` |
| UI screens | `Screen` enum in game.rs; L: `title.rs` + `title/`, `options_ui.rs`, `multiplayer.rs`, `pack_screen.rs`, `settings_shell.rs` (Horizon's settings screens on it: `extras_ui.rs`, `graphics_ui.rs`, `controller_ui.rs`), `screen_draw.rs`, `ui.rs` (widgets), `hud.rs`, `hud_layout.rs`, `hud_overlays.rs`, `hud_kit/` (Horizon overlays' `Room` and `Plate`, the collapsed page strip `tabs` that Shaders' screens use, [docs/ui-kit.md](docs/ui-kit.md)), `pad_glyph.rs` (controller glyphs, by what the button does), `chat.rs`, `tab_list.rs`; G: `startup_menu.rs`, `loading_screen.rs` (the loading screen, the pre-game `App`), `screens.rs` (pause, death), `options.rs`, `lan_screen.rs`, `chat.rs`, `sign_edit.rs`, `books.rs`, `creative_screen.rs` |
| Multiplayer client | L: `net.rs` (network thread); G: `remote.rs` (`fn handle_packet`, `fn tick_remote`), `remote_*.rs` (mob data, objects, projectiles, vehicles, stations) |
| LAN host / server side | L: `lan/` (`conn.rs` handshake + login threads, `hub.rs` + `hub/` play state, `chunks.rs`, `entity.rs`, `tracking.rs`, `guest.rs`, `playerdata.rs`); G: `lan.rs` (`LanHost`), `network/lan_actions/` (`turns.rs`: `fn as_guest`, `tick.rs`: `fn tick_guests`), `lan_*.rs` per feature, `lan_entities.rs` (tracked entities) |
| Integrated server | G: `integrated.rs` (the server thread), `levels.rs` (one level per dimension, `fn tick_server`), `server_travel.rs` |
| Dedicated server | G: `dedicated.rs`, `dedicated/status.rs`; crates `horizon-server-files`, `horizon-server`, `horizon-anticheat` (L: `lan/anticheat.rs`) |
| Save and load | L: `save.rs` (level.dat, regions, playerdata), `entity_nbt.rs`, `options_file.rs`; `horizon-client-core/src/block_entity_nbt.rs`; G: `saving.rs`, `persist.rs` (entities), `world_session.rs` (opening and closing a world); `crates/horizon-world/src/{region.rs,datafix/}`, `crates/horizon-core/src/nbt.rs` |
| Input | game.rs `fn window_event`; G: `play_keys.rs` (`fn key_pressed`), `play_input.rs` (`fn play_mouse`, toggled keys, the hotbar wheel); crate `horizon-client-input` (`keybinds.rs`, `keymapping.rs`, `key_names.rs`, `mouse.rs`, `input.rs`, `interaction_input.rs`, `gamepad.rs`, `clipboard.rs`); L: `osk.rs`; G: `pad.rs`, `pad_menu.rs`, `pad_craft.rs`, `pointer_capture.rs` (mouse look, pointer grab) |
| Audio | L: `audio.rs` (mixer), `sound_engine.rs`, `sound_events.rs`, `music.rs`, `ambience.rs`; G: `music.rs`, `subtitle_hud.rs` |
| Resource packs | L: `packs.rs` + `packs/`, `pack_screen.rs`, `atlas_sources.rs`, `sprites.rs`, `mipmap.rs`, `model_library.rs`, `font.rs`, `mobs/cem.rs` + `mobs/cem_anim.rs` (OptiFine/EMF entity models and their animations); `horizon-client-core/src/lang.rs`; crates `horizon-assets`, `horizon-optifine`; G: `packs.rs` |
| Agent control | L: `control.rs` (socket), `bin/horizon-ctl/`; G: `control.rs`, `agent.rs`, `events.rs`, `ui_control.rs` ([docs/agent-control.md](docs/agent-control.md)) |
| Mods | crate `horizon-mod-package`; G: `modding/mods/mod.rs` (content and mod load), `modding/mods/standalone.rs` (standalone model registration); L: `mod_content.rs`, `mod_list.rs` |
| Horizon extras, settings | L: `extras.rs`, `graphics.rs`, `settings.rs`, `minimap.rs`; G: `interface/extras.rs` + `interface/extras/` (each extra's glue: `mouse_tweaks.rs`, `sorting.rs`, `pvp.rs`, `reconnect.rs`), `options.rs` |

## How `Game` is organised

- `struct Game` (game.rs) keeps the core loop and fields. Each `game/<area>/<feature>.rs` (or
  `<feature>/mod.rs`) adds an `impl Game<'_>` block and state types. Logical modules remain direct
  children of `game`; an area's `.game-area` marker enables build-derived declarations.
- Find a feature's code: `grep -rn "fn tick_<thing>\|fn <verb>_<thing>" crates/horizon-client/src/game*`;
  the file is under `game/<area>/`. Find its callers in game.rs's `fn tick` / `fn tick_world`.
- Fields that say which mode runs: `remote` (this window is a client of a server: multiplayer, LAN guest, or
  single player on its server thread), `lan` (`LanHost`: this world is open to the network), `integrated`
  (the window's own server thread), `serving` (this `Game` *is* that server thread), `options.dedicated`.
  `fn tick` returns `tick_remote()` when `remote` is set; a server ticks its levels (`fn tick_world`).
- A server's players run through the shared gameplay code: `as_guest` swaps the guest's
  state (position, inventory, mode, mining, item use) into `Game` for one call (`game/network/lan_actions/turns.rs`), and
  `levels.rs` swaps whole dimensions in the same way. Changes reach clients through the block log, level
  events and the entity tracker (`lan/tracking.rs`, `game/network/lan_entities.rs`).
- Shared read-only data: `Resources` (`res`: blocks, items, recipes, mob types, biomes) and `PackResources`
  (`pack`), both in game.rs, their methods in `game/resource_tables.rs`.

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
cargo run --profile fast -p horizon-game -- --resources .local/horizon-client/26.3 --world T               # server thread
python3 tools/lan_e2e.py --check <name>                  # host on its server thread, guest is a server client
python3 tools/parity.py dedicated_server server_entities server_player_damage
```

## Docs worth knowing

[docs/parity-workflow.md](docs/parity-workflow.md) (runner, gate, shards, scopes) ·
[docs/native-client.md](docs/native-client.md) (options, controls, horizon-ctl) ·
[docs/server-levels.md](docs/server-levels.md) (levels per dimension) ·
[docs/rust-parity.md](docs/rust-parity.md) (what is ported) · [docs/known-issues.md](docs/known-issues.md) ·
[docs/release-flow.md](docs/release-flow.md) · [docs/README.md](docs/README.md) (index).
