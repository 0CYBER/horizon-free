# Horizon documentation

To play, use [the launcher](../launcher/README.md); the project site is [playhorizon.dev](https://playhorizon.dev). These pages are for developers.

## Build and run

- [Build from source](../README.md#build-from-source): your own copy of the game, resource preparation, `cargo run`.
- [Native client](native-client.md): options, controls, the dedicated server, `horizon-ctl` scripting, packaging, architecture.
- [Graphics](graphics.md): the Simple and Advanced graphics settings, the graphics benchmark and what it measured.
- [Benchmark](benchmark.md): Horizon against vanilla 26.3 and vanilla with performance mods, on the same machine, world, path and settings: what is measured, how it is kept fair, how to reproduce it.
- [Extras](extras.md): optional zoom, minimap, FPS and keystroke overlays and auto-reconnect, all off by default.
- [Launcher](../launcher/README.md): installs, updates and starts builds; its CLI and MCP server; releasing it.
- [Opt-in data collection](telemetry.md): the consent, exactly what is sent, how it is stored and sent.
- [Benchmark report format](benchmark-report.md): the versioned JSON every benchmark writes (game, launcher, `tools/perf_bench.py`), the local log and, with consent, the upload.
- Menus: the title, world list, create world, multiplayer, options and pause screens share the launcher's design tokens and fonts (`crates/horizon-theme`), drawn by `ui_theme` over a live frozen backdrop (`game/backdrop.rs`, blurred and graded by `post.rs`). Options → Interface → Menu style switches to vanilla's screens (Classic, kept in `horizon.txt`; `HORIZON_MENU_STYLE=horizon|classic` overrides it, and scripted runs default to Classic). [Title](media/menu-title.png) ([webm](media/menu-title.webm)), [worlds](media/menu-worlds.png), [options](media/menu-options.png), [pause](media/menu-pause.png).

## Parity

- [Rust rewrite status](rust-parity.md): completion criteria, how to reproduce the validation, and per-area scope.
- [Parity workflow](parity-workflow.md): `tools/parity.py`, the gate, other machines, merging.
- Area notes: [block behaviours and ticking](parity/blocks.md), [section meshing](parity/meshing.md).
- [Decompilation and research lab](research-lab.md) and [decompilation status](decompilation-status.md): the local game inspection workspace the harnesses build on.

## Architecture: world generation

- [World generation](world-generation.md): the pipeline from noise to features.
- [Noise generation](noise-generation.md), [compiled density functions](density-functions.md), [structure density](structure-density.md), [terrain filling](terrain-filling.md).
- Biomes: [lookup](biome-lookup.md), [temperatures](biome-temperatures.md).
- Surfaces: [material surfaces](material-surfaces.md), [material conditions](material-conditions.md), [rule libraries](material-rule-libraries.md), [vertical anchors](vertical-anchors.md), [value providers](value-providers.md).
- [Carving](carving.md), [heightmaps](heightmaps.md), [feature placement](feature-placement.md).

## Architecture: world data and simulation

- [World data](world-data.md): chunk storage, palettes, region files.
- [Block-state codecs](block-state-codecs.md), [scheduled ticks](scheduled-ticks.md).
- [Collision geometry](collision-geometry.md): shapes, collision, movement and fluids.
- [Face visibility](face-visibility.md): which model faces the mesher emits.

## The earlier Java track (no longer developed)

- [Java performance packs](java-packs.md), with [Vulkan on your Mac](vulkan.md), [collector comparisons](collectors.md), [profiling](profiling.md), [process memory](process-memory.md) and [tick state](tick-state.md); [server quickstart](server-quickstart.txt).
