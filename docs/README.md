# Horizon documentation

To play, use [the launcher](../launcher/README.md). These pages are for developers.

## Build and run

- [Build from source](../README.md#build-from-source): your own copy of the game, resource preparation, `cargo run`.
- [Native client](native-client.md): options, controls, the dedicated server, `horizon-ctl` scripting, packaging, architecture.
- [Launcher](../launcher/README.md): installs, updates and starts builds; its CLI and MCP server; releasing it.

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
