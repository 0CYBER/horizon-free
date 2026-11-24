# Session handoff: GPU density parity and worldgen

> **Historical session notes (2026-11-21).** This was written while the
> experimental branch was still local. The committed code, probes and fixtures
> live in [#19144](https://github.com/horizon-rust/horizon/pull/19144); the one
> claimable next step is
> [#19146](https://github.com/horizon-rust/horizon/issues/19146). Every
> **[VERIFIED]** result below was measured on that branch (base `eaa7da4fb`);
> it must be repeated on current `main` and is not a current result for `main`.

## Read first, in order

- **[VERIFIED]** `docs/gpu-noise-parity-progress.md` — cumulative phase log, exact probe results, completed gates, and resume anchor.
- **[VERIFIED]** `docs/gpu-noise-parity.md` — commands, locked R1–R6 arithmetic contract, coverage limits, and measured adapter results.
- **[VERIFIED]** `docs/radv-divergence.md` — RADV mismatch localization, explicit-FMA outcome, and `RADV_DEBUG=noopt` constraint.
- **[VERIFIED]** `docs/density-ir-design.md` — supported exporter subset, blocker categories, and semantics required before extending the IR.
- **[VERIFIED]** `docs/gpu-adapter-enablement.md` — hardware adapter inventory and recorded per-adapter probe results.

> The probe and fixture paths below are **not in this checkout**. They are in
> [#19144](https://github.com/horizon-rust/horizon/pull/19144). Read them there
> until the code is ported.

- **[IN #19144]** `crates/horizon-core/src/density/graph/compiler.rs` — source of truth for `export_to_program`, `export_blockers`, and blocker mappings.
- **[IN #19144]** `crates/horizon-core/src/density/graph/mod.rs` — graph-node definitions traversed by the exporter.
- **[VERIFIED]** `crates/horizon-world/src/worldgen/router.rs` — production router construction entry point and roots.
- **[IN #19144]** `crates/horizon-client/examples/density_program_probe.rs` — R1–R6 CPU/WGSL ladder and fixed-volume probe.
- **[IN #19144]** `crates/horizon-client/examples/wgpu_perlin_probe.rs` — Perlin and Smeared Perlin leaf probe.
- **[IN #19144]** `crates/horizon-client/examples/wgpu_lattice_probe.rs` — coordinate, wrap, permutation, and gradient-index probe.
- **[IN #19144]** `crates/horizon-client/examples/wgpu_density_interp_probe.rs` — trilinear interpolation and density blend probe.
- **[IN #19144]** `crates/horizon-client/examples/wgpu_density_graph_probe.rs` — compiled spline/blend composition probe.
- **[IN #19144]** `crates/horizon-client/examples/wgpu_sqrt_probe.rs` — separate sqrt behavior diagnostic.
- **[IN #19144]** `crates/horizon-client/examples/wgpu_adapter_inventory.rs` — visible WGPU adapter inventory.
- **[IN #19144]** `crates/horizon-client/examples/radv_divergence_probe.rs` — strict versus explicit-FMA backend diagnostic.
- **[IN #19144]** `crates/horizon-client/examples/density_program_world.rs` — deterministic fixed-volume fixture used by density-program checks.
- **[IN #19144]** `crates/horizon-client-lib/Cargo.toml` — example registrations and current WGPU-related manifest changes; inspect before adding dependencies or targets.

## Locked contract

- **[VERIFIED]** For manually constructed density-program R1–R6, use explicit f32 FMA: Rust `f32::mul_add` paired with WGSL `fma()` for gradient dots, fade, interpolation, spline, and blend; preserve the other operations' stated order and endpoint-specialized lerp branches.
- **[VERIFIED]** llvmpipe `llvmpipe (LLVM 22.1.8, 256 bits) (Cpu, GL)`, WGPU/Naga 30.0.1, `shader_f64=false`, default pipeline options: R1–R6 each had 0/60,000 bit mismatches.
- **[VERIFIED]** AMD `AMD Radeon Graphics (radeonsi, rembrandt, ACO, DRM 3.64, 7.2.0-1-cachyos-bore-lto) (Other, GL)`, WGPU/Naga 30.0.1, `shader_f64=false`, default pipeline options: R1–R6 each had 0/60,000 bit mismatches.
- **[VERIFIED]** AMD `AMD Radeon Graphics (RADV REMBRANDT) (IntegratedGpu, Vulkan)`, WGPU/Naga 30.0.1, `shader_f64=true`, default pipeline options: R1–R6 maximum absolute difference was `2.3841858e-7` over the 60,000-sample ladder; contract limit is `2.4e-7`, not bit equality. R1 had 8,986 mismatches; R6 had zero difference.
- **[VERIFIED]** `RADV_DEBUG=noopt` did not improve RADV results: explicit-FMA R1 remained 8,986/60,000 mismatches, maximum absolute difference `2.3841858e-7`.
- **[VERIFIED]** These are results for named adapters, this toolchain/configuration, and deterministic probe inputs. They do not establish backend-independent error bounds, bit-exactness across backends, results for other adapters/driver versions, or parity for the production vanilla router.
- **[VERIFIED]** The separate arbitrary-bit `sqrt` probe had 232 differences against its narrowed-f64 oracle on the recorded llvmpipe run; it is not part of the Perlin/density-program acceptance contract.

## What exists

- **[VERIFIED]** Exported IR node kinds: `Perlin`, `Constant`, `Lerp`, `Spline`, `Offset`, `Add`, `Mul`, and `Blend`; the production graph exporter currently emits only constants, recursively unwrapped references, Add/Multiply, and Lerp whose children all export.
- **[VERIFIED]** `DensityCompiler::export_to_program(FunctionId) -> Option<DensityProgram>` returns an optional representable program; `export_blockers(FunctionId)` reports each reachable unsupported definition once, with its node kind and reason.
- **[VERIFIED]** `wgpu_perlin_probe.rs` checks Perlin and Smeared Perlin outputs against CPU-precomputed lattice inputs.
- **[VERIFIED]** `wgpu_lattice_probe.rs` checks coordinate, wrap, permutation, and gradient-index stages.
- **[VERIFIED]** `wgpu_density_interp_probe.rs` checks trilinear interpolation and density blend.
- **[VERIFIED]** `wgpu_density_graph_probe.rs` checks compiled spline/blend composition over CPU-generated leaves.
- **[VERIFIED]** `density_program_probe.rs` checks the explicit-FMA R1–R6 ladder, including the trivial custom-router fixed volume.
- **[VERIFIED]** `density_program_world.rs` supplies the deterministic density-program volume fixture.
- **[VERIFIED]** `radv_divergence_probe.rs` compares strict and explicit-FMA arithmetic and diagnoses backend divergence.
- **[VERIFIED]** `wgpu_sqrt_probe.rs` separately measures sqrt behavior; `wgpu_adapter_inventory.rs` lists visible WGPU adapters.
- **[VERIFIED]** `radv_divergence_probe.rs` is 450 lines and `density_program_probe.rs` is 395 lines; keep probe entry files below 500 lines.
- **[VERIFIED]** No feature-gated production WGPU worldgen route exists; exporter/IR currently serve probes and do not replace CPU terrain generation.

## Historical blocker (superseded by Phase 48)

- **[HISTORICAL]** When first written, vanilla overworld router export was
  unmeasured because the prepared registry/settings bundle at
  `.local/horizon-client/26.3` was absent.
- **[HISTORICAL]** Phase 48 later measured it on the WIP branch: 229 reachable
  `final_density` nodes, 136 supported, 93 blockers, still **0 of 8** router
  fields exported. That is a branch measurement, not a result for `main`.
- **[OPEN]** Repeat the export on current `main` as part of the rung-2 port and
  record the numbers in `gpu-noise-parity-progress.md`.

## Unsupported node list from `export_blockers`

- **[VERIFIED]** `NoiseKind`: Noise, OffsetNoise, BlendedNoise, EndIslands.
- **[VERIFIED]** `NeedsContext`: WorldField, BlendDensity, Slice, Interpolated, Surface. `Cache` is unwrapped to its child.
- **[VERIFIED]** `UnsupportedOperation`: Spline, Gradient, Round, Power, Choice, `Sqrt`/`Log` Unary, IntervalSelect, DistanceToPoint.
- **[VERIFIED]** `BoundSampler`: already-bound sampler definitions.
- Phase 48 also exports `Clamp`, the remaining `Unary` kinds and every `Binary`
  kind; see the progress log for the current disposition.

## Ordered tasks

1. **[OPEN] Port rung 2 to current `main` first.** Take the rung-2 files from
   [#19144](https://github.com/horizon-rust/horizon/pull/19144), then require
   the 0/60,000 llvmpipe gate before any later rung. If the gate fails, stop
   and fix the emitter or the mirror; do not start rungs 3–7.
2. **[OPEN] Measure the actual vanilla router export on current `main`.** Locate and load the default overworld density registry/settings using the existing loader; identify its `final_density` graph and call `export_blockers` and `export_to_program`. Touch only the smallest relevant probe/test and `docs/gpu-noise-parity-progress.md` (update `docs/density-ir-design.md` only if coverage decisions change); do not alter the four protected files. Gate: record the complete blockers and export result. If export fails, stop before production wiring and make the A2 scope decision below based on observed blockers.
3. **[OPEN] Extend one IR node family only if task 2 proves it is required.** Choose one of `NoiseKind`, `NeedsContext`, `UnsupportedOperation`, or `BoundSampler`; state the concrete vanilla blocker, required inputs/semantics, and independent CPU expected-value test before coding. Touch `crates/horizon-core/src/density/program.rs`, its compiler/tests, the focused probe, and progress/design docs as needed. Gate: independent CPU expected values and the existing named-adapter probe contract pass; stop if semantics or vanilla inputs cannot be represented without guessing.
4. **[OPEN] Add production GPU routing only after vanilla export succeeds.** Draft the smallest feature-gated `gpu-worldgen` route and compare a deterministic vanilla overworld volume with the CPU path on the named adapters. Touch only an unprotected feature/module seam, focused probe/fixture, manifest if a feature is required, and parity docs; do not edit protected files. If integration requires a protected file, stop and record the exact seam blocker. Gate: report voxel threshold flips and numeric errors per adapter under the locked contract; do not tune tolerance to pass.
5. **[OPEN] Run final gates after changes.** Run `cargo test -p horizon-core -p horizon-world`, `cargo build -p horizon-client`, `cargo fmt --all -- --check`, and `git diff --check`; rerun the focused parity harnesses for touched code and update the progress log with exact results. Gate: report each result and any remaining open router/adapter coverage.

## Constraints

- **[VERIFIED]** No `git reset`, `git checkout --`, `git stash`, or snapshot reverts.
- **[VERIFIED]** Protected files: `crates/horizon-core/src/noise/perlin.rs`, `crates/horizon-core/src/density/interpolation.rs`, `crates/horizon-world/src/worldgen/generator.rs`, `crates/horizon-world/src/worldgen/terrain.rs`.
- **[VERIFIED]** Probe entry files must remain below 500 lines.
- **[VERIFIED]** Add no new dependencies unless feature-gated.
- **[VERIFIED]** Every parity claim names the adapter, backend, versions, and arithmetic contract.
- **[VERIFIED]** Tag every claim **VERIFIED**, **INFERRED**, or **OPEN**; use **INFERRED** only for a clearly labeled deduction, never as an observed result.
- **[VERIFIED]** Required final gates: `cargo test -p horizon-core -p horizon-world`; `cargo build -p horizon-client`; `cargo fmt --all -- --check`; `git diff --check`.
- **[VERIFIED]** Leave unrelated untracked files untouched; Phase 23 storage cleanup was explicitly skipped.

## Autonomy rules

- **[VERIFIED] A1 — Resource limit:** on a limit, stop safely, preserve current edits, and append a resumption entry to `docs/gpu-noise-parity-progress.md`.
- **[VERIFIED] A2 — Scope decision:** escalate the evidence-based choice of which unsupported IR node family to extend first if the vanilla router does not export; do not guess semantics or implement multiple families speculatively.
- **[VERIFIED] A3 — Repeated failure:** if the same blocking failure occurs three consecutive times, stop repeating it, record exact evidence and the required external precondition in the progress log, then report the blocker.
- **[VERIFIED] A4 — Completion:** continue authorized work through implementation, scoped verification, documentation, and final report; do not stop at a plan.
- **[VERIFIED]** Do not stop for plan approval or baseline re-confirmation; proceed autonomously through gates whose preconditions are met.

## Done means

- **[VERIFIED]** Target: bit-exact R1–R6 on the tested llvmpipe and radeonsi GL configurations, maximum absolute error no greater than `2.4e-7` on the tested RADV Vulkan configuration, and vanilla overworld GPU worldgen behind a feature flag with measured CPU/GPU volume results.
- **[VERIFIED]** Original cross-hardware bit-exactness is architecturally out of reach as a WGPU/WGSL guarantee: backend arithmetic differs, and the measured RADV path is not bit-exact under the locked contract. Preserve the bounded per-adapter target; do not claim universal exactness.
- **[OPEN]** Vanilla-router export and production worldgen parity remain unproven until the prepared bundle is available and the ordered gates pass.

## Resume protocol

- **[VERIFIED]** `docs/gpu-noise-parity-progress.md` is the resumption anchor. If a session limit interrupts work, append rather than truncate an entry naming the task in flight, commands run, exact results, and the precondition for resuming. Preserve existing history and working-tree changes.

## Handoff status — 2026-11-21 (Phase 48, stopped mid-rung 2)

- **[VERIFIED]** Phase 48 rung 1 (`Cache` unwrap) is done and gated on the WIP
  branch: overworld `final_density` blockers fell 109 → 93 (supported 120 →
  136); still 0 of 8 fields export. `cargo test -p horizon-core --lib` passes.
- **[VERIFIED]** Phase 48 rung 2 IR, CPU mirror, graph mapping and the
  IR→SPIR-V emitter (`crates/horizon-core/src/density/spirv.rs`) compile.
- **[VERIFIED]** (follow-up audit) The emitter's `Constant`, `Perlin` and a
  rung-2 module now assemble with `spirv-as --target-env vulkan1.3` and pass
  `spirv-val`. `OpCompositeExtract` uses literal indices, access-chain IDs are
  spelled with `%`, the block member has its own `v3i` pointer type, the
  `Point` stride is 64 bytes, and the WGSL helpers use a bit-pattern NaN test
  instead of the non-existent `isNan`. `density_program_probe` runs
  0/60,000 on llvmpipe.
- **[OPEN]** Rung 2 is *not* gated: no probe dispatches the emitted module and
  the 0/60,000 llvmpipe run for the emitted rung-2 module has not happened. Do
  not describe it as passing.
- **[OPEN]** The tree is based on an old `main` (`eaa7da4fb`); upstream has
  moved far ahead. Port the work instead of assuming the branch merges.
- **[VERIFIED]** The work is published on branch `wip/gpu-density-exact-tier`
  in [#19144](https://github.com/horizon-rust/horizon/pull/19144); the docs are
  in [#19148](https://github.com/horizon-rust/horizon/pull/19148).
- Next step: port rung 2, then require **0/60,000 on llvmpipe** before starting
  rungs 3–7 in the order recorded in `gpu-noise-parity-progress.md`, then Phase
  49 (startup self-test and the per-(device, driver) allowlist).
