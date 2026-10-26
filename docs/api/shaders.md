# Shader API

Horizon has no shader packs today: the earlier implementation was removed on 2026-10-24. Two things will replace it,
designed separately: a **Horizon shader API**, described here, and a clean-room loader for the existing pack format
(its own spec; it is not part of this API). Both sit on the same renderer hooks, agreed between the two.

## Overview: what a shader author will be able to do

Write WGSL against documented stages of Horizon's renderer, with typed inputs, options the player can change, a
reload as soon as a file is saved and an overlay that shows what each pass costs and draws. The renderer keeps
everything that makes it fast (culling, batching, LOD, the mesher); a pack changes how surfaces and light look, not
how the world is fed to the GPU.

## The design

- **A pack** is a folder or a zip with `horizon-shaders.toml` (id, name, version, the `api` line, the stages it
  uses, its options) and WGSL files. No preprocessor and no other shading language: WGSL is what wgpu validates and
  compiles for Vulkan, Metal and DirectX 12.
- **Stages, not pipelines.** A stage is a function with a fixed signature that the renderer calls inside its own pass,
  for example a terrain material (`fn material(input: TerrainInput) -> Surface`), the lighting of the visible
  surfaces, the sky, an optional shadow pass, and a chain of full-screen post passes ending in the frame. A pack
  that leaves a stage out gets vanilla's look for it.
- **Typed inputs.** Each stage receives structs (the frame: time, camera, sun and moon, weather, fog; the surface:
  position, normal, block and its light) whose WGSL is generated from the Rust types the renderer uploads. The
  generated reference lists every field with its unit and range, and a test fails when it drifts.
- **Options are WGSL `override` constants.** The manifest declares each option (on/off, a number in a range, a choice)
  with its default; the game shows them on the pack's settings screen and sets them when it builds the pipeline. No
  text substitution, and every option combination is validated the same way.
- **Hot reload.** The game watches the pack's folder. A saved file is validated with naga and the stage rebuilt; on an
  error the last good version keeps drawing and the error, with its file and line, is shown in the overlay.
- **Debug overlay.** GPU time per stage and pass (timestamp queries), the targets each pass writes (show any of them
  full-screen), the pack's options and its compile errors.
- **Budgets.** A pack declares the targets and passes it needs; the renderer refuses more than its limits, and the
  benchmark ([benchmark-report.md](../benchmark-report.md)) records the pack and its cost, so "fast" stays measured.
- **Off by default.** Without a pack the renderer is vanilla's look, and no pack code runs.

## The pack format (schema 1)

A pack is a folder, or the same files zipped as a `.hshader`, with `horizon-shaders.toml` at its root
(crate `horizon-shader-api`; the sample with every key is
`crates/horizon-shader-api/tests/samples/schema-1/`):

```toml
schema = 1                 # the manifest's format; a reader refuses schemas newer than it knows
id = "soft_light"          # [a-z0-9_]{1,64}: the settings key and the registry id
name = "Soft Light"
version = "1.0.0"
api = "0.1"                # the shader API line the WGSL is written for
description = "..."        # optional, like authors and license
shared = ["common.wgsl"]   # optional: WGSL put before every stage's file, in order

[sky]                      # each stage is optional, but a pack has at least one
file = "sky.wgsl"

[material]
file = "material.wgsl"

[[post]]                   # full-screen passes, in the order they run (8 at most)
file = "post/grade.wgsl"
name = "Colour grade"      # optional: the GPU time overlay's label

[[options]]                # 64 at most; `id` is the WGSL `override` it sets
id = "saturation"
name = "Saturation"
type = "float"             # bool, float (min, max, step), int (min, max), choice (values)
default = 1.1
min = 0.0
max = 2.0
step = 0.05

[budget]
gpu_ms = 1.5               # optional: the GPU time the pack aims to add (0 to 16 ms)
```

An option's `override` has the type of its kind: `bool`, `f32`, `i32`, or `u32` for a choice (the index of the
value). The pack holds only the manifest, the `.wgsl` files it names (each used, UTF-8, 256 KiB at most),
`README.md`, `LICENSE` (`.md`, `.txt`) and `icon.png`, under plain paths, 8 MiB in all. A `.hshader` is built
the same way every time (files in path order, fixed dates), so a pack's SHA-256 depends only on its files.

## Guide, reference and examples

- [shaders-reference.md](shaders-reference.md): every stage's signature, input and output structs, resources and
  helpers, the frame's fields with their units, option types and limits, generated from `crates/horizon-shader-api`.
- [The guide](../shaders.md): making a pack with `horizon-shader new`, `check` and `pack`; the templates and the
  example pack (`crates/horizon-shader-api/packs/golden_hour`).

## Stability

Not started. The first stages will be experimental (opt-in), then preview once a reference, an example and a test
pack exist ([stability.md](stability.md)).

## Not there yet

Option values other than the defaults (the three stages run, with hot reload), then the overlay, lighting and shadow
stages.

## Changelog

- 2026-10-26: the material stage in the game: terrain's surfaces through the pack's `material`.
- 2026-10-26: the sky stage in the game: the pack's `sky` gets vanilla's colour along each ray.
- 2026-10-26: the post stage in the game (`HORIZON_SHADER_PACK`, hot reload with the last good pack kept).
- 2026-10-25: `horizon-shader` (`new`, `check`, `pack`), three templates, the example pack and the guide.
- 2026-10-25: the check (naga validation, signatures, options, budgets, SPIR-V/Metal/HLSL translation).
- 2026-10-25: the stages of API 0.1 (sky, material, post), their inputs and the generated reference.
- 2026-10-25: the pack format, schema 1: `horizon-shaders.toml` and the `.hshader` package.
- 2026-10-24: the earlier shader pack code was removed; this API is designed from scratch.
