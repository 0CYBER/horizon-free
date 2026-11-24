# Horizon documentation

To play, use [the launcher](../launcher/README.md); the project site is [playhorizon.dev](https://playhorizon.dev). These pages are for developers. What is being done now and next, the work board and how to help: [the hub](https://playhorizon.dev/hub) ([ROADMAP.md](../ROADMAP.md), [roadmap/README.md](../roadmap/README.md) for keeping it current).

## Start here and workflow

- [AGENTS.md](../AGENTS.md) and [Architecture](architecture.md): where each part of the code lives, how `Game` is organised, which harness checks which area.
- [Build from source](native-client.md#run-from-a-checkout): your own copy of the game, resource preparation, `cargo run`.
- [How Horizon is built](how-horizon-is-built.md): who writes Horizon, how changes are checked, and why it is safe to run.
- [Layers](layers.md): the engine, platform and Horizon dependency boundary, how Horizon crates plug in, and how CI checks it.
- [Forking](forking.md): the `horizon-extras` feature that holds Horizon's own extras (cosmetics, Friends, its settings pages…), building without them, and Microsoft sign-in rules for forks.
- [Release flow](release-flow.md): pull request checks, merging and the automatic Nightly releases that reach players.
- [Versioning](versioning.md): how releases are numbered from 0.2 (`v0.2.x`, `launcher@0.2.x`, Nightly tags), and how installs from before 0.2 get there.
- [Operations](operations.md): running Horizon by hand when no agent runs and the maintainer's machines are off: merging, releases, offline runners, the mods registry, Kira, the official server, security.
- [Security](security.md): how Horizon protects code, CI and automatic releases against untrusted input while work is performed by AI agents.
- [Scoped source reviews](security-reviews.md): JSON records in `docs/security-reviews/` capturing what a reviewer inspected at a specific commit and file hash.
- [Build times](build-times.md): `tools/build_times.py` measures cold and incremental builds per crate and the critical path, writes a JSON report and a table, and compares two reports; the first numbers, Windows and Linux.
- [Known issues](known-issues.md): player-facing issues listed in each game release and removed as pull requests fix them.

## Client, graphics and features

- [Native client](native-client.md): options, controls, musl Linux installation, the dedicated server, `horizon-ctl` scripting, packaging, architecture.
- [Graphics](graphics.md): the graphics settings screen (pages, search, descriptions), the graphics benchmark and what it measured.
- [HDR and render passes](hdr-passes.md): what a pass on the world's picture (upscaling, anti-aliasing, frame generation, post effects, captures) keeps so HDR works, and how to check it.
- [Controllers](controllers.md): playing with PlayStation, Xbox and Nintendo Switch Pro controllers, Steam Deck built-in controls, analog movement, and the input layer.
- [Extras](extras.md): optional zoom, minimap, FPS and keystroke overlays, dynamic lights and auto-reconnect, all off by default.
- [PvP & HUD](pvp.md): optional client settings for PvP players (custom crosshair, block outline, hit colour, HUD widgets), fair-play boundaries, and why each is off by default.
- [Cosmetics](cosmetics.md): optional Horizon-only cosmetics, emotes and badges: rules, service protocol and settings.
- [Play with Friends](play-with-friends.md): invite codes, encrypted tunnels and the relay for sharing a local world without port forwarding.
- [Narrator](narrator.md): vanilla's text-to-speech (Ctrl+B): the voices per platform, how screens record what they narrate, `horizon-ctl narration`, and where it still differs from vanilla.
- [Resource packs](resource-packs.md): everything a 26.3 resource pack can change, whether Horizon honours it, and the proof.
- [Reusing resource-pack overlays](reuse-pack-overlays.md): historical benchmark baseline and notes for the previous materialising overlay loader (superseded by in-place resolution in [resource-packs.md](resource-packs.md)).
- [Realms availability](realms-availability.md): vanilla's check that this client may use Realms, the five results, how a screen reads it (one `Pending`, kept), its caching, and how to test a consumer.
- [Translations](translations.md): language coverage across ~85 languages and human review of machine-written strings.

**Menus:** every screen is vanilla's (title, world list, create world, multiplayer, options, pause), except Horizon's graphics settings, drawn in the design system's Blocks palette. Horizon's own settings hang off vanilla's Options the way mods add theirs, each in one place: **Video Settings...** opens Horizon's graphics settings screen (every video option, the graphics extras and the shader packs), **Controls → Controller Settings...** the controller settings screen in the same style, **Mods...** and **Horizon Settings...** take the last two slots of the Options grid (Horizon Settings opens Extras, PvP & HUD, Splitscreen, Replay Studio and AI Agent, each a vanilla options list). The title screen's and pause menu's Friends buttons open Join a Friend and Play with Friends ([play-with-friends.md](play-with-friends.md)).

## Dedicated server

- [Host a server](hosting.md): install `horizon-server`, prepare the game files, first run, `server.toml`, the commands, the web panel, backups, services and Docker; what works today and what doesn't yet.
- [For server owners](owners.md): every `server.properties` key next to its `server.toml` field.
- [A server's levels](server-levels.md): how the dedicated server and single player's server thread tick, save and travel between dimensions.
- [The anticheat](anticheat.md): the dedicated server's own checks, on by default (`combat` and `ground` act, `timer` and `prediction` only log) and set in `server.toml`'s `[anticheat]`: violation levels, setback, alerts, kicks, the log, and how to write a check.
- [The chat filter](chat-filter.md): the dedicated server refuses hate slurs in chat, message commands, signs, anvil names and book titles: the list and its normalising, mutes, temporary bans, `[chat_filter]`.
- [Velocity forwarding](velocity-forwarding.md): the player-information exchange behind a Velocity proxy and how it was checked.
- [Pregeneration](pregeneration.md): `/pregen`, generating, lighting and saving an area ahead of the players.
- [Natural spawning on the parallel tick](parallel-natural-spawning.md): how `HORIZON_PARALLEL_TICK` proposes chunk spawns in parallel and commits them serially.
- [Parallel mob tick](mob-islands.md): reach and acceptance requirements for the opt-in `upstream-parallel-mobs` mob-island loop.
- [Survival multiplayer](smp.md): the official server's `smp` native mod (economy, market, trades, games of chance, spawn, homes, teleport requests, warps, land claims) and the server hook points it needs; not part of Horizon's own server.
- [Hosting a server: design](server.md): `horizon-server`, the design and the plan: install, `server.toml`, the CLI, the web panel, backups, services, Docker, and the bot benchmark against vanilla.
- [The next server: design](server-next.md): Horizon's dedicated server against vanilla and Pumpkin (measured), the targets, parallel ticking that keeps vanilla's order, the network front, the Horizon fast path, the plugin API and the plan.

## Mods, shaders and APIs

- [Horizon APIs](api/README.md): every surface other programs build on (mods, shaders, agent control, servers, the launcher, launch, content, data formats), each with its guide, generated reference, stability and changelog, and the [stability and versioning policy](api/stability.md).
- [The modding book](modding/src/introduction.md): content mods (data), WebAssembly mods and native mods (Rust), how to make, install and publish them ([entry page](mods.md)). [Native mods](modding/src/how-to/native-mods.md): how the launcher builds them ([contract & SDK notes](native-mods.md)). [Hook counterparts](https://github.com/horizon-rust/horizon/tree/main/crates/horizon-mod-vocab/src/hooks): the NeoForge and Fabric catalog conventions.
- [Writing the modding book](modding-book.md): Diátaxis structure, preprocessors, prose rules and wasm fixture checks for contributors to `docs/modding/`.
- [Block pick results](native-block-clone-item.md): how `BlockPart::clone_item_stack` handles creative/middle-click pick results across local and guest paths.
- [Exact-build native dylib probe](native-dylib-probe.md): developer probe checking exact-build dynamic linking against a prebuilt SDK.
- [Shader packs](shaders.md): Horizon's own WGSL shader packs, how to make and check one (`horizon-shader`).
- [Upgrading a 26.2 data pack](datapack-upgrade.md): `horizon-port-pack upgrade-datapack` rewrites a 26.2 worldgen pack in 26.3's formats, and how the rewrite is checked against vanilla.
- [Modpacks](modpacks.md): porting whole packs (ATM10 and smaller): import, inventory, carry-over, an agent per mod with a harness against the Java original, coverage, and how "10x" is measured.
- [Agent control](agent-control.md): what AI agents can drive in the launcher and the game, and the design of the control socket, `horizon-ctl` and MCP.
- [horizon-ctl commands](horizon-ctl-commands.md): every command `horizon-ctl` takes, its CLI arguments, and the corresponding MCP tools for driving the game.

## Performance and telemetry

- [Benchmark](benchmark.md): Horizon against vanilla 26.3 and vanilla with performance mods, on the same machine, world, path and settings: what is measured, how it is kept fair, how to reproduce it.
- [Benchmark report format](benchmark-report.md): the versioned JSON every benchmark writes (game, launcher, `tools/perf_bench.py`) and the local log.
- [Server against vanilla](performance/server-vs-vanilla.md): Horizon's server, vanilla 26.3, Paper and Fabric + Lithium under the same bots at 1, 10, 25 and 50 players: tick time, CPU, memory, chunks and startup.
- [Server against Pumpkin](performance/server-vs-pumpkin.md): Horizon's server and Pumpkin (Rust) under the same bots at 0, 20, 60 and 100 players: tick time, CPU, memory, chunks, join; where Horizon loses, and how to run it again.
- [Hot-path performance trends](hot-path-trends.md): the dispatch-only job that counts instructions and allocations of the tick, commands, chunk saving and mod dispatch on fixed workloads, and how to compare two runs.
- [Memory](memory.md): what holds the game's memory (`horizon-ctl memory`, the `alloc-census` heap census and its measured cost, resident memory in benchmark reports, heap profilers on probes).
- [Performance platform (Spec P)](perf-platform.md): design for the `.hzperf` capture format, the `horizon-perf` dev tool and the web viewer.
- [JFR analysis for Java comparisons](profiling.md): profiling vanilla Java runs with Java Flight Recorder when comparing hot paths.
- [Opt-in data collection](telemetry.md): consent switches (all off by default), what is sent when enabled, the anonymous "I'm playing" ping, *Show me as Horizon*, and in-game bug reports.

## Launcher and platforms

- [Launcher](../launcher/README.md): installs, updates and starts builds; its CLI and MCP server; releasing it.
- [Launcher commands](launcher-commands.md): every command of `horizon-launcher`, CLI arguments, and the tools `horizon-launcher mcp` offers for driving launcher actions.
- [Moving launcher data](launcher-data-location.md): moving launcher data, worlds and build caches to another drive (including spaces and Unicode paths on Windows).
- [The launcher's mod browser](mod-browser.md): searching Modrinth and the Horizon registry for mods, modpacks, resource packs and shader packs, compatibility checks before install, and checksum verification.
- [Launcher resource preparation](launcher-preparation.md): compiling Java exporter classes into release bundles, verifying local Minecraft assets against `research-lock.json`, and multi-lane export workflows.
- [Microsoft account sign-in](microsoft-account.md): signing players in with their Microsoft account in the launcher, joining online-mode servers, the owner gate, and the OAuth device-code flow.
- [Nix and NixOS](nix.md): the flake's packages: server, engine, prepare, client and launcher, and the `services.horizon-server` NixOS module.
- [Windows allocators](windows-allocators.md): mimalloc defaults, the optional `windows-segment-heap` Cargo feature for Win32 heap APIs, and how to validate the executable manifest.
- [Windows ARM64](windows-arm64.md): building and running native Windows ARM64 executables for `aarch64-pc-windows-msvc`, cross-compilation, and release packaging.

## Parity

- [Horizon status](status.md): player-visible gaps open and the subsystem-by-subsystem harness score table.
- [Rust rewrite status](rust-parity.md): detailed subsystem parity log and differential coverage notes.
- [Parity workflow](parity-workflow.md): `tools/parity.py`, the gate, other machines, merging.
- [Port ledger](port-ledger.md): `tools/port_ledger.py`, every vanilla method next to the Rust functions that port it, both ways, with the gaps where a port isn't wired like vanilla.
- [Paper event parity](paper-parity.md): local trace comparisons, scenarios and tolerances against Paper 26.3.
- [Paper and Bukkit porting ledger](paper-ledger.md): mapping of Paper/Bukkit plugin services to Horizon native-mod routes and limits.
- [Fabric API mapping ledger](fabric-ledger.md): mapping of Fabric transfer, registry and data APIs onto Horizon's NeoForge-shaped parts and hooks.
- [CI runners](ci-runners.md): which jobs run on GitHub's runners and which on ours (the CI VPS, playhorizon, the Mac mini), the `SELF_HOSTED_CI` switch, and where a fork's pull request may run.
- Area notes: [block behaviours and ticking](parity/blocks.md), [section meshing](parity/meshing.md), [redstone machines tick by tick](parity/redstone-diff.md), [the same seed in vanilla and Horizon](parity/world-compare.md).
- [GPU noise parity](gpu-noise-parity.md): the WGSL Perlin probe, the locked per-adapter FMA contract and its coverage limits.
- [RADV divergence](radv-divergence.md): explicit-FMA tolerance, backend probes and the remaining scope.
- [Density-program IR design](density-ir-design.md): the density-graph export subset, the CPU/GPU lowering plan and representability limits.
- [GPU adapter enablement](gpu-adapter-enablement.md): host inventory, WGPU adapter selection and the device-passthrough blocker.
- [GPU density progress](gpu-noise-parity-progress.md): phases 1–48, measured gates and the resume anchor for the exact-or-off tier.
- [GPU density handoff](session-handoff.md): the short read-first list, the locked contract and the open next step.
- [Version ports](version-ports.md): how a new Minecraft release is detected, diffed against the pinned one and ported, and what stays manual.
- [Reading vanilla](parity-workflow.md#reading-vanilla): the local decompiled sources and bytecode queries the harnesses build on.

## World generation and data

- [World generation](world-generation.md): the pipeline from noise to features. Each stage's vanilla comparison is its harness (`tools/test_<name>_parity.py`); `python3 tools/port_ledger.py where Class.method` finds the Rust port of a vanilla method.
- [World height](world-height.md): each level's shape from its dimension type, data packs' taller levels.
- [World data](world-data.md): chunk storage, palettes, region files.

## Design notes

- [Design system](design-system.md): shared tokens and interface rules for the launcher, website and Horizon-only game overlays.
- [HUD kit](ui-kit.md): the pieces Horizon's own HUD overlays share: `Room`, which keeps the controller notice and splitscreen's join strip below the boss bars, and `Plate`, the box behind their text.
- [Linear light for HDR and shader packs](design/hdr-linear-light.md): the proposed shader API 0.2 colour contract and frame format for HDR output.
- [The shader API 0.2 frame](design/shader-api-frame.md): proposed: where a 0.2 pack's stages, post chain and tone curve run, who owns anti-aliasing and coverage, and room for clouds, mods, LabPBR and ray tracing.
- [Live mod reloading](design/mod-hotload.md): loading, enabling and updating mods from inside the game without a restart.
- [Wasm group faults](design/wasm-group-faults.md): what a wasm dependency group does after a trap or an interruption, and how interruption counts guest work.
- [Mod registry v2](design/registry-v2.md): project kinds, metadata, installation locations and the registry rollout.
- [Scoped whole-stack rollback](design/stack-snapshot.md): transaction-scoped, same-place savepoints for internals mods; the real transaction lifetime must be confirmed before implementation, and provider-owned places are initially refused.
