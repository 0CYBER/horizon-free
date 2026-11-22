# GPU world-generation parity progress

This log records the probe stages and their limits. It is cumulative across
work sessions.

## Phase 0 — State confirmation — DONE

- Commands: inspected `git status`, checked the four protected core source files
  with `git diff`, checked the parity guide, counted the existing example, and
  ran `cargo run -p horizon-client-lib --example wgpu_perlin_probe`.
- Results (VERIFIED): probe exists in the workspace; the four protected source
  files have no diff; `docs/gpu-noise-parity.md` exists. Adapter was llvmpipe
  (LLVM 22.1.8, Cpu, Gl; fp64=false). Perlin and Smeared Perlin each had
  0/60,000 mismatches; smoothstep had 0/60,000. Sqrt had 232/60,006 mismatches.
  No compiler warnings. The probe source is 763 lines, above the per-example
  limit, so diagnostics will be factored as needed.
- Decision: use `horizon-client-lib` as the example owner, as Cargo reports.
- Next-phase precondition: G1 can be isolated in a new small example and may
  change probe files/docs only; core source remains untouched absent a concrete
  failure.

## Phase 1 — GPU coordinate-to-lattice parity (G1) — DONE

- Command: `cargo run -p horizon-client-lib --example wgpu_lattice_probe`.
- Contract: f32 coordinates and offsets narrowed from the Perlin table; exact
  f32 wrap, shift, floor/cell, fraction, x-neighbor permutation and eight
  corner-gradient hash indices. Inputs include negative/non-integral values,
  near-integers and magnitudes through 100,000,000.
- Results (VERIFIED): llvmpipe OpenGL; all 14 reported axis/stage comparisons
  had 0/60,000 mismatches, total stage mismatches 0. First divergence: none.
  The 387-line example builds and runs without warnings.
- Compile fixes: corrected one WGSL layout descriptor delimiter, changed a
  slice-to-array mapping to explicit axes, unwrapped readback mapping, and gave
  the fraction array an explicit length. All were resolved before the passing
  run; no core files changed.
- Decision: this establishes parity for f32 coordinate processing only; it
  does not establish parity with Horizon's f64 worldgen coordinate path.
- Next-phase precondition: G2a can independently exercise trilinear corner
  ordering and interpolation with a narrowed-f32 reference. G2b is permitted
  only if G2a is DONE.

## Phase 2 — Density interpolation parity (G2a) — DONE

- Command: `cargo run -p horizon-client-lib --example wgpu_density_interp_probe`.
- Contract: compare WGSL against the narrowed-f32 CPU operation order for the
  scalar trilinear path in `horizon-core/src/density/interpolation.rs` and the
  blend-factor path in `horizon-core/src/density/selection.rs`. Cases cover
  8 distinct signed corners, three interpolation factors, blend endpoints
  including signed zero and one, a subnormal factor, near-endpoint factors,
  and 60,000 deterministic samples total.
- Results (VERIFIED): llvmpipe OpenGL; trilinear corner interpolation
  0/60,000 mismatches; density blend factor 0/60,000; total 0. The 264-line
  example built and ran without warnings. First divergence: none.
- Decision: accept zero-bit-difference for these operations on the tested
  adapter. This probe covers scalar cell interpolation and density lerp; it
  does not cover the separate bulk sampler's repeated-Y accumulation or graph
  composition.
- Next-phase precondition: G2b is permitted because G2a is DONE. It must remain
  within the probe-size limit and may compare only graph behavior supported by
  existing public constructors/references; no core edits are authorized absent
  a concrete divergence.

## Phase 3 — Full density graph (G2b) — PARTIAL

- Command: `cargo run -p horizon-client-lib --example wgpu_density_graph_probe`.
- Scope: the Rust side builds and evaluates Horizon's actual compiled density
  subgraph: three CPU Perlin leaf fields, a cubic spline over one field, and a
  LerpSampler blending the other two. WGSL receives the leaf values and
  evaluates the spline and blend. This is deliberately not a GPU NoiseRouter
  or full terrain graph.
- Results (VERIFIED): current probe run on llvmpipe had 0/60,000 ordinary
  compiled-spline mismatches and 0/60,000 compiled-spline-plus-blend
  mismatches. Its explicit-FMA diagnostic variant had 2,048/60,000 spline and
  1,270/60,000 blend mismatches (3,318 total); first diagnostic divergence was
  sample 66, input `[-0.16800003, 0.00025177147, 0.037311606]`, where CPU was
  `0x3edd5a8e` and GPU was `0x3edd5a8d`.
- Divergence history (VERIFIED): the initial scalar-output WGSL form reported
  422/60,000 spline and 304/60,000 blend mismatches. The independently written
  Rust spline expression matched Horizon's compiled sampler at its first
  failure. After expanding the WGSL diagnostic output to include an explicit
  FMA variant, the ordinary path reported zero; this codegen sensitivity means
  the result is not proof of a full graph contract. No tolerance was loosened.
- Decision: mark PARTIAL because NoiseRouter construction, complete graph
  composition, density caches, actual grid interpolation in the graph, and
  chunk terrain output are not evaluated by this WGSL stage. No full-graph
  tolerance is claimed.
- Next-phase precondition: G4 can identify the CPU seam and check for an
  existing GPU route; no integration code will be added if none exists.

## Phase 4 — Worldgen integration seam (G4) — DONE

- Evidence (VERIFIED): `rg -n "wgpu|wgsl|Gpu" crates/horizon-world/src/worldgen
  crates/horizon-core/src/density crates/horizon-core/src/noise` returned no
  GPU path. In `crates/horizon-world/src/worldgen/generator.rs`,
  `GeneratorWorker::generate_proto` calls the seeded pipeline; its
  `Pipeline::generate_with_context` calls `fill_noise` at line 1695 with the
  router's `FinalDensity` sampler.
- Minimum seam (VERIFIED): `crates/horizon-world/src/worldgen/terrain.rs`,
  `fill_noise_inner`, calls
  `final_density.sample_volume(context, density, volume)` at line 675, before
  aquifer and block writes. That is the narrowest point where a complete GPU
  density volume could replace CPU output and be compared before use.
- Status: DOES NOT EXIST. There is no worldgen WGPU code or GPU density buffer
  route in these crates.
- Decision (D4): acceleration integration is out of scope; no speculative
  production path was added. Continue with probes and document the seam.
- Next-phase precondition: G5 may evaluate the separate sqrt diagnostic using
  its measured input domains; it does not gate Perlin parity.

## Phase 5 — Sqrt triage — DONE (separate from parity gate)

- Evidence (VERIFIED): the probe's 232/60,006 mismatches consist of 127
  positive subnormal inputs whose GPU result is zero, and 105 negative inputs
  whose Rust f64 reference is NaN but GPU result is finite. Positive normal
  inputs have 0 mismatches. This is the synthetic arbitrary-bit-input sqrt
  diagnostic only; it does not call the fade or Perlin gradient path.
- Spec check (VERIFIED): the current WGSL spec gives `sqrt` domain [0,+infinity]
  and accuracy inherited from `1.0 / inverseSqrt(x)`; the f32 `inverseSqrt`
  bound is 2 ULP. WGSL also permits subnormal inputs/outputs of accuracy-listed
  operations to flush to zero. Negative inputs are outside sqrt's specified
  domain. Source: https://gpuweb.github.io/gpuweb/wgsl/ §§15.7.2, 15.7.4,
  17.5.36 and 17.5.58.
- Recommendation (not applied): keep sqrt outside the Perlin gate. For a
  future relevant GPU call site, test finite nonnegative normal inputs against
  a ≤2 ULP band; test subnormals under an explicit flush-to-zero policy; do not
  use negative inputs as a conformance assertion. Leave current diagnostic
  inputs intact so it continues to expose backend behavior.
- Next-phase precondition: enumerate adapters; rerun parity probes on each
  hardware adapter if one is available.

## Phase 6 — Hardware adapter availability — SKIPPED-HW

- Command: `cargo run -p horizon-client-lib --example wgpu_adapter_inventory`.
- Result (VERIFIED): one adapter is visible: llvmpipe (LLVM 22.1.8, Cpu, Gl),
  with shader_f64=false. No hardware GPU adapter was available in this runtime,
  so cross-vendor hardware parity is skipped.
- Gate: the llvmpipe checks do not establish Vulkan, Metal, DX12, or hardware
  GPU parity. Run the documented probes on each supported target adapter.

## Phase 7 — Contract and rerun documentation — DONE

- Added standalone commands and scopes for lattice, interpolation, graph, sqrt,
  and adapter inventory to `docs/gpu-noise-parity.md`.
- Contract: exact f32 output-bit agreement for the sampled Perlin and narrowed
  density operations on a named adapter. This is not an all-adapter guarantee
  and does not cover CPU f64 worldgen coordinate generation. Sqrt has its own
  WGSL-derived policy, documented separately; its arbitrary-bit stress result
  is not a Perlin failure.
- Probe organization (VERIFIED): all example entry files are under 500 lines;
  Perlin CPU arithmetic diagnostics live in a sibling helper module. Example
  commands are owned by `horizon-client-lib`; WGPU, pollster and bytemuck were
  already regular dependencies, so the manifest only registers examples.
- Final reruns (VERIFIED): Perlin 0/60,000 and Smeared Perlin 0/60,000;
  lattice stages total 0/60,000; interpolation stages total 0/60,000; ordinary
  graph spline and spline+blend each 0/60,000. The explicit-FMA graph diagnostic
  separately reports 2,048 spline and 1,270 blend mismatches. Sqrt remains
  232/60,006, separately analyzed in Phase 5. Probe build emitted no warnings
  after removing the extracted helper's unused function.
- Scope limit: G2b remains PARTIAL, and Phase 6 is SKIPPED-HW. These probes do
  not implement GPU-accelerated world generation or prove complete terrain
  output parity.
- Final gate (VERIFIED): `cargo test -p horizon-core -p horizon-world` passed.
  Horizon Core reported 123 passed and 3 ignored; Horizon World reported 338
  passed and 2 ignored, with its integration tests and doctests completing
  successfully. `cargo fmt --check` and `git diff --check` passed.

## Phase 8 — FMA parity contract — DONE on llvmpipe

- Repro (VERIFIED): sample 66 has x=`0xbe2c0833` (-0.16800003). The
  strict-expression CPU spline returns `0x3edd5a8e`; explicit CPU
  `f32::mul_add` returns `0x3edd5a8d`. WGSL explicit `fma()` returned the
  latter on llvmpipe. The previous one-bit mismatch compared these unlike
  arithmetic variants.
- Results (VERIFIED): the WGSL ordinary-expression path (no explicit FMA call
  in source) matched the CPU strict-expression reference in 0/60,000 spline
  and 0/60,000 spline+blend cases. WGSL permits backend contraction, so this
  is an observed comparison, not a portable no-contraction guarantee. Explicit
  `fma()` WGSL matched the equivalent CPU `mul_add` spline and blend references
  in 0/60,000 each. The gate is met for both tested variants on this adapter.
- Chosen contract (VERIFIED for llvmpipe): explicit `f32::mul_add` paired
  with WGSL `fma()`, following D2. The strict-expression variant remains a
  second diagnostic and also matched on this adapter. No fast-math or custom
  floating-point flags are set by the Rust probe; wgpu pipeline compilation
  uses default options. WGSL §15.7.5 permits backend fusion and has no
  contraction-disable control, while §17.5.32 permits `fma()` expansion.
  Therefore neither result is a backend-independent guarantee. Source: W3C
  WGSL, https://www.w3.org/TR/WGSL/.
- Gate (VERIFIED): `cargo test -p horizon-core -p horizon-world` passed
  (Core 123 passed/3 ignored; World 338 passed/2 ignored, integration tests
  passed). `cargo build -p horizon-client` completed with no warnings.

## Phase 9 — Full production NoiseRouter coverage — PARTIAL

- Evidence (VERIFIED): the production API exposes `NoiseRouter::from_json`,
  `NoiseRouter::compile`, and eight `RouterSamplers` roots. This probe now
  exercises the compiled spline and lerp samplers through `sample_volume` on
  a 60,000-cell synthetic volume, with CPU Perlin-backed leaf fields and WGSL
  composition. The strict-expression and explicit-FMA variants report zero
  mismatches on the tested adapter.
- Limitation (VERIFIED): no `.local` prepared 26.3 registry/settings bundle is
  present in this checkout, so the actual overworld NoiseRouter cannot be
  instantiated here. Further, DensityCompiler exposes opaque `DensitySampler`
  trait objects, not an exportable shader IR. D4 is respected: no production
  graph refactor was made. The router API itself is already public/testable;
  the missing piece is production input data plus a WGSL lowering interface.
- Coverage matrix (INFERRED from probe scope): `Noise`/shifted-noise,
  `BlendedNoise`, `EndIslands`, `WorldField`/`BlendDensity`, unary/binary,
  gradient, round/power/clamp/choice/interval selection, cache, slice,
  interpolated and surface nodes are unmeasured by this WGSL probe. No mismatch
  counts are claimed for those node types. The exercised node mismatch counts
  are: Perlin-backed leaves as inputs (leaf values are CPU generated; GPU
  comparison is not leaf generation), spline 0/60,000 and lerp composition
  0/60,000 under each tested arithmetic variant.
- Gate (VERIFIED): `cargo run -p horizon-client-lib --example
  wgpu_density_graph_probe` passed; the CPU/reference result uses bulk
  `sample_volume`, not only scalar sampling. Phase remains PARTIAL because it
  is not the production router graph.

## Phase 10 — Hardware backend retry — SKIPPED-HW

- Adapter inventory (VERIFIED): wgpu enumerated exactly one adapter:
  llvmpipe (LLVM 22.1.8, 256 bits), Cpu, Gl, shader_f64=false. No integrated,
  discrete, Vulkan, Metal, or DX12 hardware adapter is available in this
  environment.
- Reruns (VERIFIED): Perlin 0/60,000, Smeared Perlin 0/60,000; lattice
  0/60,000 total; interpolation 0/60,000 total; spline, spline+blend,
  explicit-FMA spline, and explicit-FMA blend each 0/60,000; sqrt 232/60,006.
  All were run on the single llvmpipe adapter. Hardware retry is skipped.
- Gate (VERIFIED): `cargo test -p horizon-core -p horizon-world` passed
  (Core 123 passed/3 ignored; World 338 passed/2 ignored, integration tests
  passed). `cargo build -p horizon-client` completed without warnings.

## Phase 11 — Minimum GPU worldgen path — PARTIAL (not implemented)

- Seam review (VERIFIED): `fill_noise_inner` receives only
  `&dyn DensitySampler`, `SamplerContext`, and a `DensityVolume`, then fills a
  CPU `&mut [f32]` through `sample_volume`. The sampler is an opaque trait
  object; the density graph compiler does not expose an exportable program or
  WGSL lowering. Router root IDs are not enough to reconstruct the compiled
  graph after sampler construction.
- Decision: no feature flag, dependency, fake identity shader, or CPU-then-GPU
  copy was added. Such a path would still evaluate every density node on CPU
  and would not meet the mission's GPU worldgen requirement. The default CPU
  path and production code remain unchanged.
- Proposal only (INFERRED): a future implementation needs a stable density
  program representation shared by the compiler and GPU backend, plus explicit
  treatment for unsupported nodes, cache semantics, seeded noise inputs,
  interpolation, and sampler context. This is a production architecture change
  beyond the permitted test/example-only seam exposure in D4.
- Gate: `cargo build --features gpu-worldgen` was not run because that feature
  and path do not exist. Phase 11 exit criterion is unmet; no accelerated
  worldgen claim is made.

## Phase 12 — End-to-end terrain parity — PARTIAL (blocked by Phase 11)

- GPU terrain volume (OPEN): no GPU worldgen path exists, so a same-input,
  same-adapter CPU/GPU generated terrain volume cannot be compared. No voxel
  mismatch count or first divergent voxel is claimed.
- Available evidence (VERIFIED): the Phase 9 synthetic density volume compares
  compiled CPU `sample_volume` with WGSL spline/lerp output, all exact over
  60,000 cells on llvmpipe. This is a graph subset and is not chunk terrain.
- CPU terrain tests (VERIFIED): `cargo test -p horizon-world` passed, including
  existing terrain fill-noise behavior tests. They do not invoke WGPU.

## Phase 13 — Documentation and final report — DONE (partial mission)

- Updated `docs/gpu-noise-parity.md` with the two tested FMA variants, their
  sample-66 distinction, the adapter-specific contract limitation, current
  coverage, and the absence of a production GPU path.
- `docs/api/shaders.md` was not changed in this continuation: the compute
  probes are not part of Horizon's shader-pack API. `docs/README.md` already
  indexes the guide.
- Final state: Phase 8 DONE on llvmpipe; Phase 9 PARTIAL; Phase 10 SKIPPED-HW;
  Phase 11 PARTIAL/not implemented; Phase 12 PARTIAL; Phase 13 DONE. Exact
  WGPU-accelerated production worldgen is not achieved.

## Hardware adapter reachability — C5 / BLOCKED at runtime passthrough

- H0 inventory (VERIFIED): exact command output is in
  `docs/gpu-adapter-enablement.md`. Host-visible hardware is AMD Rembrandt
  Radeon 680M (`1002:1681`); `amdgpu`, a Radeon Vulkan ICD, and sysfs
  `renderD128` are present. `/dev/dri` and `/dev/dxg` are absent in the task
  environment. `systemd-detect-virt` returned `container-other`.
- H1 classification (VERIFIED): C5 CONTAINER/VM. The system has a GPU and
  driver, but this runtime has no mapped DRM device node. This is not NO_GPU,
  GPU_NO_DRIVER, or GPU_NO_VULKAN.
- H2 (VERIFIED): the adapter inventory returned only llvmpipe under no env,
  `WGPU_BACKEND=vulkan`, `WGPU_BACKEND=gl`, and
  `WGPU_BACKEND=vulkan WGPU_POWER_PREF=high`. `vulkaninfo --summary` returned
  no physical devices. Forcing the Radeon ICD also returned no valid GPUs;
  forcing lavapipe failed because `libLLVM.so.23.1` is missing. No adapter
  names were available from vulkaninfo for its name-selector variant.
- Probe selector support (VERIFIED): the five probe entry points now use
  wgpu's environment-aware adapter initializer, so `WGPU_ADAPTER_NAME` can
  select a named adapter. Each was run with `WGPU_ADAPTER_NAME=llvmpipe` and
  selected the software adapter; results and commands are in
  `docs/gpu-adapter-enablement.md`. No hardware parity was produced.
- H3/H4/H6 (INFERRED): not applicable to the C5 classification. ICD manifests
  and the `amdgpu` driver are present, so installing a driver/ICD or recording
  the C1 no-GPU path would be incorrect. No package, driver, ICD, or runtime
  change was attempted.
- H5 (VERIFIED blocker): `/sys/class/drm/renderD128` exists but `/dev/dri`
  does not. Minimum host action: relaunch the task with the host render node
  passed through and the render GID available, or run on the host directly.
  In-container group changes cannot create the absent device mapping.
- H7 (SKIPPED-HW): no physical adapter could be opened; prior parity results
  remain llvmpipe-only. The reproducible GPU-host probe loop is in
  `docs/gpu-adapter-enablement.md`.
- H8 (DONE): adapter evidence and selection instructions are documented in
  `docs/gpu-adapter-enablement.md` and `docs/gpu-noise-parity.md`.

## Hardware adapter reachability correction — host retry completed

This append-only update corrects the preceding sandbox-only conclusion.

- Reachability (VERIFIED): the ordinary sandbox could not open hardware because
  `/dev/dri` was absent, but an approved command outside the sandbox reached
  both host adapters: AMD Radeon Graphics (RADV REMBRANDT), Vulkan,
  `shader_f64=true`; and AMD Radeon Graphics (radeonsi, Rembrandt), GL,
  `shader_f64=false`. Thus C5 describes the sandbox boundary, not a host-wide
  inability to reach hardware.
- Inventory environment variants (VERIFIED): default, `WGPU_BACKEND=vulkan`,
  `WGPU_BACKEND=gl`, and Vulkan plus `WGPU_POWER_PREF=high` each listed both
  adapters. The inventory enumerates all backends, so these variables did not
  filter that list.
- Hardware Perlin results (VERIFIED, WGPU/Naga 30.0.1, default shader
  compilation options): RADV Vulkan had 50,656/60,000 Perlin and
  48,157/60,000 Smeared Perlin mismatches; radeonsi GL had 50,296/60,000 and
  47,709/60,000 respectively. Both had 0/60,000 lattice-stage mismatches.
  Therefore hardware access is proven, but CPU/GPU Perlin output parity is
  disproven on both tested AMD backends under this arithmetic contract.
- Separate diagnostics (VERIFIED): each AMD backend had 27,315/60,000
  trilinear differences, 8,190/60,000 density-blend differences, and
  4,767/60,006 arbitrary-bit sqrt differences. The sqrt result is a separate
  floating-point issue and is not part of the Perlin mismatch count. Full
  stage-by-stage data and first-divergence samples are in
  `docs/gpu-adapter-enablement.md`.
- Production worldgen (OPEN): these probes do not provide a GPU NoiseRouter or
  terrain-volume implementation; accelerated production world generation
  remains unimplemented and unverified.

## Phase 14 — Density-program IR design — DONE

- **[VERIFIED]** Compiler path: `DensityCompiler::get_sampler` in
  `crates/horizon-core/src/density/graph/compiler.rs:47-54` calls graph
  optimization (`:73-75`) and then `compile_raw` (`:150+`). `prepare`
  (`:77-101`) rewrites references and allocates cache samplers. Its result is
  `Arc<dyn DensitySampler>`, so the production compiled sampler does not
  retain an exportable node program.
- **[VERIFIED]** Router path: `NoiseRouter::compile` in
  `crates/horizon-world/src/worldgen/router.rs:71-81` obtains eight samplers;
  generator code selects `RouterField::FinalDensity` and calls
  `sample_volume` at `generator.rs:2883-2901`. The terrain seam calls
  `final_density.sample_volume` at `terrain.rs:675`.
- **[VERIFIED]** `docs/density-ir-design.md` records every `GraphNode` kind,
  compiler dispatch location, representability, the R1-R6 schema, lowering
  choices and reasons caches/context-bound nodes cannot be flattened as-is.
- **[OPEN]** This IR is a second representation and is not automatically
  extracted from the production graph. Its representability does not imply
  general router export.

## Phases 15-16 — Density-program ladder R1-R6 — DONE for stated subsets

- Adapter/options (**[VERIFIED]**):
  `llvmpipe (LLVM 22.1.8, 256 bits) (Cpu, Gl)`, WGPU/Naga 30.0.1,
  `shader_f64=false`, default pipeline compilation options, forced for the
  run with `LIBGL_ALWAYS_SOFTWARE=1 WGPU_ADAPTER_NAME=llvmpipe`.
- **[VERIFIED]** `cargo run -p horizon-client-lib --example
  density_program_probe` passed with CPU IR vs existing `PerlinNoise` at
  0/60,000 and these CPU IR/WGSL bit mismatch counts:

  | Rung | Tested subset | Mismatches |
  |---|---|---:|
  | R1 | seeded Perlin with CPU-narrowed cell/offset input | 0/60,000 |
  | R2 | constant plus lerp at alpha 0, 0.375 and 1 | 0/60,000 each |
  | R3 | one cubic spline segment over the Perlin leaf | 0/60,000 |
  | R4 | f32 coordinate offset over the Perlin sample | 0/60,000 |
  | R5 | add, multiply and blend composition | 0/60,000 |
  | R6 | custom router root `add(0.125, 0.125)` | 0/60,000 |

- **[VERIFIED]** The R1 permutation comes from the existing seeded
  `PerlinNoise` table; this IR does not implement or reorder seed draws.
  Composition shaders materialize the base Perlin leaf in a prior dispatch.
- **[VERIFIED]** R3 initially differed at sample 5 (`0x3f16d432` CPU,
  `0x3f16d434` GPU) when the leaf was inlined into the spline. A local WGSL
  variable retained that difference. Splitting the Perlin leaf into its own
  GPU dispatch removed the optimizer-sensitive fusion; the final staged
  R1-R6 run is bit-exact on the named llvmpipe configuration above.
- **[VERIFIED]** The entry example is 363 lines, below the 500-line limit;
  its terrain-volume fixture is in the 168-line sibling module. No new crate
  dependencies or changes to the four protected files were needed.

## Phase 17 — Fixed-volume comparison — DONE for custom trivial router

- **[VERIFIED]** A custom test router was decoded and compiled through the
  existing `NoiseRouter` / `DensityCompiler`; its `final_density` is
  `0.125 + 0.125`. This is a valid trivial test definition, not the vanilla
  overworld preset.
- **[VERIFIED]** On seed 0, chunk (0,0), fixed volume 16×16×16 at block origin
  `(0,0,0)`, the recorded value from `fill_noise` (which calls
  `fill_noise_inner`) matched the direct compiled sampler at 0/4,096; direct
  sampler matched IR CPU at 0/4,096; and IR CPU matched IR GPU at 0/4,096.
  All 4,096 resulting voxels used the default solid block. No divergent voxel
  or node path was found.
- **[OPEN]** The fixed volume does not cover any vanilla overworld router node
  or prove that the production density graph can be exported.

## Phase 18 — Adapter coverage — SKIPPED-HW

- **[VERIFIED]** The probe was explicitly forced to llvmpipe for the ladder
  and volume verification. No adapter inventory or hardware recheck was run
  for this phase, per the run instruction.
- **[VERIFIED]** An earlier unforced initial R1 invocation selected RADV
  REMBRANDT and reported 50,656/60,000 leaf mismatches before the adapter was
  forced to llvmpipe. This incidental single-stage result is not full hardware
  coverage; no R2-R6 or fixed-volume hardware run is claimed.

## Phase 19 — Docs and final gates — IN PROGRESS

- **[VERIFIED]** Updated `docs/density-ir-design.md`,
  `docs/gpu-noise-parity.md`, and `docs/README.md`; this progress log is
  append-only. The guide names the adapter, backend, versions and compilation
  options for every new parity claim.
- **[VERIFIED]** First requested `cargo test -p horizon-core -p horizon-world`
  run passed before the R2-R6 extension (Core 124 passed, 3 ignored; World
  338 passed, 2 ignored; integration tests passed). A post-extension final
  gate is still required.
- **[OPEN]** Final warning-free no-feature build, post-extension crate tests,
  formatting and whitespace checks remain to be rerun after all source edits.

## Phase 19 final gate — DONE

- **[VERIFIED]** Post-extension `cargo test -p horizon-core -p horizon-world`
  passed: Core 124 passed / 3 ignored; World 338 passed / 2 ignored, plus
  integration tests and doctests.
- **[VERIFIED]** `cargo build -p horizon-client` completed without warnings.
- **[VERIFIED]** The final forced-llvmpipe `density_program_probe` run printed
  WGPU/Naga 30.0.1, GL, `shader_f64=false`, default pipeline compilation
  options and zero mismatches at R1, R2 alpha 0/interior/1, R3, R4, R5 and
  R6. Phase 17 reported zero differences for each 4,096-cell comparison.
- **[VERIFIED]** `cargo fmt --check`, `git diff --check` and the whitespace
  scan of new source/docs passed. `density_program_probe.rs` is 380 lines and
  its sibling fixture is 168 lines.
- **[VERIFIED]** Protected `perlin.rs`, `interpolation.rs`, `generator.rs`,
  and `terrain.rs` remain unchanged. No dependency was added. The unrelated
  `context.md`, `opencode/` and `opencode-old/` paths were left untouched.
- Final scope: IR and custom trivial-router volume parity are exact on the
  named llvmpipe configuration; production vanilla-router lowering and GPU
  wiring remain open. The earlier accidental RADV R1 result is recorded under
  Phase 18; no full hardware ladder is claimed.

## Phase 20 — RADV divergence diagnosis — TOLERANCE

- **[VERIFIED]** Adapter and toolchain for all Phase 20/21 results:
  AMD Radeon Graphics (RADV REMBRANDT), IntegratedGpu, Vulkan; WGPU/Naga
  30.0.1; `shader_f64=true`; default pipeline compilation options. The R1
  deterministic input set has 60,000 samples. The original probe reproduced
  50,656/60,000 strict Perlin mismatches and 48,157/60,000 Smeared Perlin
  mismatches.
- **[VERIFIED]** Corrected R1 strict-expression comparison (`a*b+c` CPU and
  ordinary WGSL expression) reported 50,626/60,000 mismatches, ULP buckets
  8,831 at 1 ULP, 28,485 at 2–16 ULP, 13,310 above 16 ULP, max 339,581 ULP,
  maximum absolute difference `2.4288893e-6`. First ten divergent indices:
  0 (coords `[16777215,-64,-16777216]`, `3e0aa9b6` vs `3e0aa9bd`, 7 ULP);
  1 (`[-148843.82426466868,-514535.96010367596,116969.59836338938]`,
  `3ca77770` vs `3ca777bb`, 75); 2 (`[-2498.729408213203,992.072354753408,
  -3090.7534142158956]`, `3e9ee5b8` vs `3e9ee5b5`, 3); 4
  (`[-11.140257711135117,-4.042030229092616,2.42543600801082]`,
  `bf0a7d42` vs `bf0a7d44`, 2); 5 (`[4875153.039905464,24752816.976813372,
  20945011.244827006]`, `3f11f7e2` vs `3f11f7df`, 3); 6
  (`[178428.5713528273,-793174.9296823546,481875.3020447939]`,
  `be8c7d94` vs `be8c7d96`, 2); 7 (`[292,1646,4076]`, `be1e8dd1` vs
  `be1e8dc3`, 14); 8 (`[-0.881715762490725,-0.20812117550979659,
  -0.5516580583545505]`, `3e1150a1` vs `3e11509e`, 3); 9
  (`[-10.658895073247916,-9.356739887275198,6.588473577729914]`,
  `be7ba2a6` vs `be7ba29b`, 11); 10 (`[19157975.575668093,23917355.258813024,
  24532875.51681367]`, `bed41ccd` vs `bed41ccc`, 1).
- **[VERIFIED]** Explicit CPU `f32::mul_add` with WGSL `fma()` reported
  9,002/60,000 mismatches (3,588 at 1 ULP, 4,927 at 2–16, 487 above 16;
  max 13,345 ULP; max absolute `2.3841858e-7`). Its first ten mismatches were
  indices 10, 33, 42, 60, 78, 81, 96, 111, 113 and 123; ULP distances were
  2, 1, 3, 2, 1, 1, 3, 1, 1 and 32. **[VERIFIED]** CPU strict IR matched
  `PerlinNoise` at 0/60,000.
- **[VERIFIED]** RADV lattice probe: zero mismatches at wrap/shift/floor-cell/
  fraction, permutation and corner-gradient gather stages. Naga's strict
  SPIR-V has separate multiply/add instructions; explicit-FMA SPIR-V has
  `OpExtInst Fma` at explicit call sites. This rules out Naga contracting the
  strict source during SPIR-V emission, but does not inspect later RADV machine
  code.
- **[VERIFIED]** Denormal test: `x=2^-49` gives CPU `x^3=0x00000004` and
  strict fade `0x00000028`; a CPU reference flushing `x^3` to zero returns
  zero. Both RADV Vulkan and llvmpipe GL returned zero and matched the FTZ
  reference. **[INFERRED]** FTZ is not the cause of the RADV-only R1 mismatch;
  it is a shared permitted behavior. The separate sqrt drift remains unrelated.
- **[VERIFIED]** Phase 20.4 verdict: **TOLERANCE** for the measured ladder and
  sample set, not bit-exact hardware parity. The largest RADV absolute error
  across R1–R6 is `2.4288893e-6`, so the recorded empirical bound is `2.5e-6`.
  Strict contract R1 has 50,626 mismatches; explicit FMA reduces this to 9,002
  but does not eliminate them. The evidence points to backend floating-point
  rounding/fusion behavior; a driver or Naga bug is not established.

## Phase 21 — Tolerance ladder and fixed volume — DONE for tested subsets

- **[VERIFIED]** RADV REMBRANDT/Vulkan, WGPU/Naga 30.0.1, `shader_f64=true`,
  default options: R1 50,626/60,000 (max abs `2.4288893e-6`); R2 interior
  49,064 (`1.5124679e-6`), alpha zero 50,626 (`2.4288893e-6`), alpha one
  0 (`0`); R3 27,347 (`1.6391277e-6`); R4 49,058 (`2.2649765e-6`); R5
  47,602 (`1.0579824e-6`); R6 0 (`0`). ULPs are unreliable near zero and are
  emitted by the probe for diagnosis; the acceptance bound is absolute.
- **[VERIFIED]** On llvmpipe (LLVM 22.1.8, 256 bits, Cpu, GL), WGPU/Naga
  30.0.1, `shader_f64=false`, default options, R1–R6 each remained
  0/60,000 bit mismatches. The tolerance definition therefore preserves
  exactness on the tested software adapter and is bounded on the tested RADV
  adapter.
- **[VERIFIED]** RADV Phase 17 custom trivial-router volume, same toolchain and
  options: fill volume vs direct sampler 0/4,096; direct sampler vs IR CPU
  0/4,096; IR CPU vs GPU 0/4,096; all 4,096 voxels selected the default solid
  block. **[OPEN]** This constant router does not test nonconstant terrain or
  vanilla NoiseRouter lowering.
- **[VERIFIED]** `docs/radv-divergence.md` records the diagnosis and scoped
  contract; `docs/gpu-noise-parity.md` now distinguishes exact llvmpipe results
  from the measured RADV tolerance.

## Phase 22 — Docs and final gates — PARTIAL

- **[VERIFIED]** `cargo test -p horizon-core -p horizon-world` passed:
  Horizon Core 124 passed / 3 ignored; Horizon World library 339 passed / 1
  ignored, plus integration and doc tests (including 8 world I/O tests passed,
  1 ignored).
- **[VERIFIED]** `cargo fmt --all -- --check` and `git diff --check` passed.
  Diagnostic entry files are below 500 lines (`radv_divergence_probe.rs` 449;
  `density_program_probe.rs` 395).
- **[VERIFIED]** `cargo build -p horizon-client` compiled the crates but failed
  at final link: `rust-lld: error: ... No space left on device`. At the time,
  the workspace filesystem had 347 MiB available. No cache or workspace files
  were removed to force a retry. This gate is therefore **OPEN**, not a code
  failure.
- **[VERIFIED]** The four protected files
  (`noise/perlin.rs`, `density/interpolation.rs`, `worldgen/terrain.rs`,
  `worldgen/generator.rs`) have no diff. No dependency was added. The unrelated
  `context.md`, `opencode/` and `opencode-old/` paths were not modified.

## Phase 22.5 — Tolerance probes — DONE

- **[VERIFIED]** On AMD Radeon Graphics (RADV REMBRANDT, IntegratedGpu,
  Vulkan), WGPU/Naga 30.0.1, `shader_f64=true`, default pipeline options,
  `RADV_DEBUG=noopt` produced the same strict R1 result (50,626/60,000,
  max absolute `2.4288893e-6`) and corrected explicit-FMA R1 result
  (8,986/60,000, max absolute `2.3841858e-7`) as the normal run. Disabling
  RADV optimization did not remove the R1 differences.
- **[VERIFIED]** On AMD Radeon Graphics (radeonsi, Rembrandt, ACO, Other,
  GL), WGPU/Naga 30.0.1, `shader_f64=false`, default options, strict R1 had
  50,296/60,000 mismatches (max absolute `2.4288893e-6`) while explicit-FMA
  R1 was bit-exact at 0/60,000. This is exactness for this adapter/path and
  sample set, not a general hardware guarantee.

## Phase 23 — Disk cleanup and build gate — SKIPPED BY USER

- The user explicitly asked to ignore this phase after freeing storage. No
  `cargo clean` or other cleanup was run. **[VERIFIED]** A disk snapshot at
  resumption showed 3.7 GiB free; earlier the failed final link had 347 MiB
  free. The Phase 23 build gate is omitted here and is covered by Phase 27's
  final build.

## Phase 24 — Explicit-FMA contract — LOCKED FOR TESTED LADDER

- **[VERIFIED]** This supersedes the Phase 20 strict-expression-only bound
  of `2.5e-6`; Phase 20 records that earlier diagnosis, while Phase 24 records
  the corrected FMA shader and final acceptance contract.
- **[VERIFIED]** The R1–R6 probe now evaluates the explicit-FMA CPU reference
  and emits matching WGSL `fma()` operations for gradient dots, fade,
  interpolation, spline and blend. It materializes the FMA Perlin leaf before
  R2/R3/R5 composition.
- **[VERIFIED]** RADV REMBRANDT/Vulkan, WGPU/Naga 30.0.1, `shader_f64=true`,
  default options, 60,000 samples: R1 8,986 mismatches/max abs
  `2.3841858e-7`; R2 interior 7,587/`1.4901161e-7`, alpha 0
  8,986/`2.3841858e-7`, alpha 1 0/0; R3 2,064/`1.7881393e-7`; R4
  4,095/`2.0861626e-7`; R5 7,045/`1.0430813e-7`; R6 0/0. Maximum over
  the tested ladder is `2.3841858e-7`, within the locked `2.4e-7` bound.
- **[VERIFIED]** On llvmpipe (LLVM 22.1.8, 256 bits, Cpu, GL), WGPU/Naga
  30.0.1, `shader_f64=false`, default options, each R1–R6 rung had
  0/60,000 mismatches. On radeonsi (AMD Rembrandt, ACO, Other, GL), WGPU/Naga
  30.0.1, `shader_f64=false`, default options, each rung likewise had
  0/60,000 mismatches.
- **[VERIFIED]** The locked rule is per adapter: bit-exact on the tested
  llvmpipe and radeonsi GL configurations, and absolute error ≤`2.4e-7` on the
  tested RADV Vulkan configuration. It is not a promise for other GPUs,
  drivers, samples, or the production router. Docs updated in the parity guide
  and RADV diagnosis.

## Phase 25 — Production router export — PARTIAL; vanilla preset unavailable

- **[VERIFIED]** Added additive `DensityCompiler::export_to_program(id) ->
  Option<DensityProgram>`, `try_export_to_program` for the first blocking
  node/reason, and `export_blockers` for a deduplicated list of every reachable
  blocker. `get_sampler` and CPU sampler compilation are unchanged.
- **[VERIFIED]** The exporter handles direct constants, reference wrappers,
  binary Add/Multiply, and Lerp when children are also representable. A
  custom `NoiseRouter` whose final density is `0.125 + 0.125` exports and its
  IR result matches the existing CPU sampler bit-for-bit. A Cache node reports
  `NeedsContext`; EndIslands reports `NoiseKind`.
- **[VERIFIED]** Unrepresented cases in the exporter match are: Bound
  definition → `BoundSampler`; Noise, OffsetNoise, BlendedNoise, EndIslands →
  `NoiseKind`; WorldField, BlendDensity, Cache, Slice, Interpolated, Surface →
  `NeedsContext`; Spline, Gradient, Unary, Round, Power, Clamp, Choice,
  non-Add/Multiply Binary, IntervalSelect and DistanceToPoint →
  `UnsupportedOperation`. References recurse into their child and propagate
  its result.
- **[VERIFIED]** `.local/horizon-client/26.3` is absent in this checkout, so
  the default vanilla overworld preset cannot be decoded here. **[OPEN]** Its
  export result and exact blocking-node list are undetermined. No claim is
  made that vanilla exports.
- **[VERIFIED]** Phase 26 was not started because its gate requires a
  successful export of the default vanilla router. No `gpu-worldgen` feature,
  dependency or production terrain wiring was added. Remaining work is to
  provide the prepared 26.3 density registry/settings, run export coverage,
  then decide whether any unsupported production semantics should be added.

## Phase 27 — Final gates — PARTIAL; Phase 25 input required

- **[VERIFIED]** With the user-freed storage, `cargo build -p horizon-client`
  completed successfully without warnings; filesystem space remained 3.4 GiB
  after linking. Phase 23 cleanup itself remained skipped as requested.
- **[VERIFIED]** After the final exporter/blocker-list changes, the full client
  build passed again without warnings; a fresh `df -h .` showed 3.7 GiB free.
- **[VERIFIED]** Final `cargo test -p horizon-core -p horizon-world` passed:
  Horizon Core 125 passed / 3 ignored; Horizon World 339 passed / 2 ignored,
  plus integration tests and doctests. The focused router-export test passed.
- **[VERIFIED]** `cargo build -p horizon-client --features gpu-worldgen` reports
  that `horizon-client` does not contain this feature. This is expected because
  Phase 26's vanilla-export precondition was not met; no feature was invented.
- **[VERIFIED]** `cargo fmt --all -- --check` and `git diff --check` passed;
  the RADV diagnostic entry file is 450 lines and the density ladder probe is
  395 lines. The four protected files remain unchanged, and no dependency was
  added.
- **[OPEN]** Phase 25 cannot establish whether the default vanilla overworld
  router exports until the prepared 26.3 density registry/settings bundle is
  available. Consequently Phase 26 and production GPU worldgen remain
  unimplemented. No vanilla volume or voxel flip count is claimed.

## Phase 28 — Vanilla router export measured — PHASE 25 BLOCKER CLEARED

- **[VERIFIED]** The prepared 26.3 bundle is now present at
  `.local/horizon-client/26.3` (374 MiB, `/.local/` is gitignored). It was
  copied from the sibling checkout
  `../horizon-android/.local/horizon-client/26.3`; no download, bootstrap or
  launcher step was run. `Projects` is mounted NTFS3, where `ln -s` fails with
  `EINVAL`, so a copy was the only option. A `find . -maxdepth 4 -name 26.3
  -not -path './target/*'` now returns that path.
- **[VERIFIED]** Added `DensityCompiler::export_coverage(id)` and the public
  `ExportBlocker` / `ExportCoverage` types (`crate::density::graph`). The
  report is the number of distinct reachable graph nodes plus every node that
  prevents a pointwise export, each with its shortest depth from the root
  (root = 0), ordered by depth then reason. `export_to_program`,
  `try_export_to_program` and `export_blockers` are unchanged. The existing
  unit test file gained `export_coverage_counts_reachable_nodes_and_attributes_blocker_depth`,
  which pins reachability through interning and the depth-1 attribution of two
  different blocker families.
- **[VERIFIED]** New probe `crates/horizon-world/examples/vanilla_router_export.rs`
  reads the `GeneratorData` bundle (`noises.json`, `densities.json`,
  `settings.json`), decodes each noise setting with the existing
  `DensityRegistry` / `NoiseRouter` loaders, and reports per router field:
  export result, reachable nodes, supported nodes, blocker count, the first
  failure, the reached family set, and blocker kinds with depth ranges.
  Command:
  `cargo build --profile fast -p horizon-world --example vanilla_router_export`
  then
  `./target/fast/examples/vanilla_router_export .local/horizon-client/26.3/worldgen --summary`.
- **[VERIFIED]** Bundle contents: 64 noises, 55 density functions, 7 noise
  settings (`amplified`, `caves`, `end`, `floating_islands`, `large_biomes`,
  `nether`, `overworld`).
- **[VERIFIED]** Router fields exporting today: `caves` 7/8 and
  `floating_islands` 7/8 (only `final_density` blocked), `end` 6/8 (`erosion`
  and `final_density` blocked), `nether` 5/8 (`temperature`, `vegetation`,
  `final_density` blocked), `overworld` 0/8, `amplified` 0/8,
  `large_biomes` 0/8.
- **[VERIFIED]** Overworld `final_density`: 229 reachable nodes, 120
  supported, 109 blockers. By family and node kind, with depth range:
  - `NeedsContext` 25: `Cache` 16 (11..=24), `Interpolated` 5 (4..=9),
    `WorldField` 3 (2..=22), `BlendDensity` 1 (6).
  - `NoiseKind` 37: `Noise` 34 (7..=22), `OffsetNoise` 2 (25),
    `BlendedNoise` 1 (14).
  - `UnsupportedOperation` 47: `Unary` 15 (3..=25), `Binary` 9 (2..=18),
    `Choice` 7 (4..=11), `Gradient` 6 (8..=20), `Clamp` 4 (14..=16),
    `IntervalSelect` 3 (18..=20), `Spline` 3 (19..=23).
  Shallowest blockers are `Binary` and `WorldField` at depth 2; the first
  `Noise` is at depth 7.
- **[VERIFIED]** No single unsupported family makes any router field export.
  Overworld `temperature`, `vegetation`, `continents`, `erosion` and `ridges`
  all need `NeedsContext` + `NoiseKind` (their `NeedsContext` blockers are
  `Cache` only, at depths 1..=4); `depth` and `chunk_surface_level` also need
  `UnsupportedOperation`; `final_density` needs all three. After
  {`NeedsContext`, `NoiseKind`} the five climate fields export (5/8); after
  all three, 8/8.
- **[VERIFIED]** Nether `temperature` and `vegetation` are each a single
  `Noise` root at depth 0, so `NoiseKind` alone would export both — the only
  single-family field win in the bundle. End `erosion` and every
  `final_density` need all three families.
- **[VERIFIED]** `Cache` is pure memoization of the child at the same
  position: `CachingDensitySampler::sample_value` returns an already stored
  `(position, value)` pair or the child's value for that position
  (`crates/horizon-core/src/density/cache.rs`). Unwrapping `Cache(input)` to
  `input` for a pointwise export is therefore an identity rule, not new
  semantics. It is unproven in this probe and is *not* implemented.
- **[OPEN]** A2 decision required before any IR family extension. The numbers
  favor `NoiseKind` first: it is the only family with a single-family field
  win (Nether climate), it is present in every blocked Overworld field, and
  with the identity `Cache` rule it reaches the 5/8 climate milestone.
  `UnsupportedOperation` has the most blockers (47/109) and the shallowest
  frontier (`Binary`@2) but unblocks no field on its own, so it cannot be
  validated end-to-end until the other two land. Caveat: real `Noise` nodes
  carry `xz_scale`/`y_scale`, while `DensityProgram::Perlin` currently
  consumes one CPU-narrowed `PerlinSample` for the whole program, so
  `NoiseKind` needs a per-leaf scaled sample model (or CPU-precomputed
  per-leaf lattice tables) in addition to the WGSL leaf that R1 already
  validates. No IR change was started.
- **[VERIFIED]** Gates for this phase: `cargo test -p horizon-core -p
  horizon-world --lib --bins` passed (Horizon Core 126 passed / 3 ignored;
  Horizon World 339 passed / 2 ignored) once the staged bundle was present;
  `cargo fmt --all -- --check` and `git diff --check` passed; the four
  protected files are unchanged. No dependency was added.

## Phase 29 — What far-LOD actually evaluates — DONE; A2 STOP

Read-only phase: no source file was changed. All line references are to HEAD
`eaa7da4fb` plus the working tree.

- **[VERIFIED]** The far-terrain path is `GeneratorWorker::lod_grid`
  (`crates/horizon-world/src/worldgen/generator.rs:2836`), called by
  `VanillaLodSampler::grid`
  (`crates/horizon-client/src/vanillagen/lod.rs:564`). The single-column
  `lod_surface` path (`crates/horizon-client/src/vanillagen.rs:744`) goes
  through `surface_sample` → `noise_column`
  (`generator.rs:1876`). Both use
  `self.router.get(RouterField::FinalDensity)`; `lod_grid` calls
  `final_density.sample_volume` once in exact mode and twice in approximate
  mode (a 64-block coarse pass, then a band-limited fine pass).
- **[VERIFIED]** The climate fields are *not* the density source. They are
  reached only through the per-column biome: `self.source.resolver(&self.climate,
  router.get(RouterField::Erosion))` and `memo_biome`
  (`generator.rs:2970`–`3058`, `2123`–`2148`), which evaluates
  temperature/vegetation/continents/erosion/ridges/depth at one quart per
  column (memoised by cell in approximate mode only).
- **[VERIFIED]** Lattice points per 16x16-column tile, exact mode
  (`width = 16 << level`, `step_xz = max(width, 4)`, `step_y = 8/16/32`,
  `ny = 384 / step_y + 1`; `nx = nz = 17` for level >= 2, `5` for level 0):

  | level | stride | step xz/y | density evaluations | biome lookups |
  |---|---|---|---|---|
  | 0 | 1 | 4 / 8 | 1,225 | 256 |
  | 2 | 4 | 4 / 8 | 14,161 | 256 |
  | 4 | 16 | 16 / 16 | 7,225 | 256 |
  | 6 | 64 | 64 / 32 | 3,757 | 256 |

  Approximate mode adds a `17 x 7 x 17 = 2,023`-point coarse pass (175 at
  level 0) and shrinks the fine band to the surface.
- **[VERIFIED]** Measured with `target/fast/examples/lod_grid_bench
  .local/horizon-client/26.3 --tiles 25 --levels 0,2,4,6`, seed 0, single
  threaded, best of two runs on the 6-core/12-thread dev machine: exact
  4.1 / 16.4 / 17.2 / 16.0 ms per tile at levels 0/2/4/6; approximate
  3.3 / 13.4 / 14.2 / 11.3 ms. Exact-mode output hashes reproduced the
  earlier session's run.
- **[INFERRED]** Density sampling dominates. Fitting a constant per-column
  cost across level 0 and level 2 (256 columns each) gives ~0.95 us per
  density evaluation and ~11.5 us per column, i.e. ~13.5 of the 16.4 ms at
  level 2, and 70–85% at levels 4/6 (bounded by the 3–5 ms approximate-mode
  saving). A precise split needs instrumentation, which this read-only phase
  forbids. No sampling profiler is usable here: `perf` and `valgrind` are not
  installed, and `eu-stack`/`gdb` cannot unwind the release build.
- **[VERIFIED]** (a) Far-LOD consumes **`final_density`**, not the overworld
  climate fields. The climate fields cover at most the 256 per-column biome
  lookups (2–3 ms of 16–17 ms per tile, **[INFERRED]**). `final_density`
  needs all three unsupported families (Phase 28), so far-LOD does **not**
  run on an exportable subset.
- **[VERIFIED]** (b) The only solid/air threshold in this path is
  `density > 0.` in `DisabledAquifer::compute_substance`
  (`crates/horizon-world/src/worldgen/aquifer.rs:85`–`91`), applied to the
  f32 density after the trilinear lattice interpolation. A 2.4e-7 absolute
  error can therefore change the output only where `|density| <= 2.4e-7`.
  **[INFERRED]** Visually that is irrelevant for far tiles: one block in one
  column, block-quantised, hundreds of blocks away. **[VERIFIED]** It does
  break bit-identical output, so the bench's full-data hash cannot be a GPU
  pass gate; a height/span tolerance gate is required.
- **[VERIFIED]** Gates: `cargo test -p horizon-core -p horizon-world --lib
  --bins` passed (126 core / 339 world, 3 and 2 ignored); no source file was
  touched in this phase.
- **[OPEN]** A2 decision required: Phase 30's stated goal (export the climate
  subset to accelerate far-LOD) does not match what far-LOD runs on. The
  options and the evidence are in the session report; no IR work was started.

## Phase 30 — Separable-leaf share of far-LOD density — DONE (G30 pass)

- Probe: `crates/horizon-world/examples/hybrid_leaf_probe.rs` (new, untracked).
  It compiles overworld `final_density`, interns the leaves, and times the real
  graph against (a) the graph on the same coarse lattice and (b) the graph with
  every separable leaf held constant.
- **[VERIFIED]** Overworld `final_density` has **37 unique post-interning
  leaves** when `minecraft:noise`, `minecraft:old_blended_noise` and the two
  non-shifted `minecraft:blended_noise` sources are counted; the interpolated
  cell size is `(4, 8)`; the coarse lattice is `5 x 49 x 5 = 1,225` points per
  chunk over `y = -64` plus 384 blocks of height. 32 of the 37 are separable
  Perlin leaves; only 3 leaves are shifted (continentalness, erosion, ridge),
  served by 2 shift sources.
- **[VERIFIED]** On the dev machine, seed 0, best of two runs: the block
  `final_density` volume costs `7.51–7.61 ms/chunk`; the same graph on the
  coarse lattice costs `4.85 ms`; the 37 leaves evaluated alone on that lattice
  (`37 x 1,225 = 45,325` evaluations) sum to `2.26 ms` (30 %); with every leaf
  forced to constant `0` the graph costs `0.39 ms` (5 %).
- **[VERIFIED]** Leaf share by ablation is therefore about **95 %** of the
  coarse-lattice graph cost, above the G30 50 % gate. **G30 PASSES**.
- **[VERIFIED]** The largest single leaf is `old_blended_noise` at
  `1.187 ms/chunk`; `cave_cheese` is `0.272 ms`.
- **[INFERRED]** The isolation sum (30 %) understates the true leaf cost
  because nested pointwise `interpolated` nodes (for example `noodle` inside
  `range_choice`) re-evaluate their leaves at up to eight corners; `Cache`
  cells are memoization only and do not dedupe. The ablation figure (95 %) is
  the one the gate uses.

## Phase 31 — wgpu separable-leaf kernel — PARTIAL (parity); throughput pass

- Files: `crates/horizon-client/examples/gpu_leaves_probe.rs` (486 lines),
  `gpu_leaves_kernel.rs` (121), `gpu_leaves_tables.rs` (125); registered in
  `crates/horizon-client-lib/Cargo.toml`. The CPU narrows each leaf per axis
  (`5 x 49 x 5`, layers included); the WGSL kernel expands the cartesian
  product and sums the per-octave Perlin layers with the bulk `add_volume`
  association `(gx*px + gz*pz) + gy*py`. Read-only: no production file was
  changed.
- **[VERIFIED]** Toolchain: WGPU/Naga 30.0.1, default pipeline options.
  Adapters reached on the dev machine: AMD Radeon Graphics (RADV REMBRANDT),
  Vulkan, `shader_f64=true`; AMD Radeon Graphics (radeonsi, rembrandt, ACO,
  DRM 3.64), GL, `shader_f64=false`; llvmpipe (LLVM 22.1.8, 256 bits), GL,
  `shader_f64=false` (reached with `WGPU_BACKEND=gl LIBGL_ALWAYS_SOFTWARE=1`).
  The Vulkan lavapipe ICD on this host is unloadable (missing
  `libLLVM.so.23.1`), so llvmpipe is the GL path only.
- **[VERIFIED]** Overworld, seed 0, `--chunks 256 --reps 3`, 31 separable
  `minecraft:noise` leaves (the probe counts noise nodes; Phase 30's 37 add
  `old_blended_noise` and non-shifted `blended_noise`): RADV GPU-only
  `6,127–6,333 chunks/s`; GPU+narrow `3,064–3,068 chunks/s`; CPU leaf baseline
  `269.3 ms` at 1 worker and `78.2 ms` at 4 workers (`~3,300 chunks/s`);
  readback `148.3 KiB/chunk` (`37.1 MiB` for 256 chunks).
- **[VERIFIED]** The mission's proto baseline is `359 chunks/s` at 4 workers,
  so the G31 2x gate is `718 chunks/s`. GPU-only and GPU+narrow both clear it
  by 4.3–8.8x. Against the probe's own 4-worker CPU leaf rate the 2x line is
  `~6,600 chunks/s`; GPU-only at `6,127–6,333` sits just under it. The gate as
  written uses the proto baseline and passes; this second comparison is
  recorded so the margin is not overstated.
- **[VERIFIED]** Parity against the production CPU leaf sampler
  (`sample_volume`, strict/vectorized, not `mul_add`):
  - llvmpipe GL, `--chunks 16 --reps 1`: **31 of 31 leaves bit-exact, 0
    mismatches**.
  - RADV Vulkan, 256 chunks: 0 of 31 bit-exact, 8,840,161 of 9,721,600 points
    differ, worst absolute `3.755e-6`, worst relative `1.0e-5`
    (`spaghetti_2d_modulator`, whose values are small), **0 sign flips**.
  - radeonsi GL, 256 chunks: 0 of 31 bit-exact, 8,817,158 mismatches, worst
    absolute `3.844e-6`, worst relative `1.0e-5`, **0 sign flips**.
- **[VERIFIED]** The residual does **not** accumulate with octave count: 32
  layers (`jagged`) show `7.8e-7` absolute, 2 layers show up to `3.8e-6`, and
  14 layers (`cave_cheese`) show `1.5e-6`. The error tracks the leaf's value
  scale, not `layers`. Both AMD backends agree to ~1 % in mismatch count,
  consistent with the same ACO contraction of ordinary WGSL expressions.
- **[VERIFIED]** The locked Phase 24 contract (bit-exact on llvmpipe and
  radeonsi GL, `<= 2.4e-7` on RADV) is an explicit-`fma()`/`mul_add` pairing.
  This kernel deliberately uses ordinary WGSL expressions, because the
  production CPU leaf is strict `add_volume`, not `mul_add`; the strict-path
  residual is the one terrain actually sees. **G31 parity is met on llvmpipe
  and is a documented tolerance on the two AMD backends**; the throughput half
  passes. No production claim of `2.4e-7` hardware parity is made.
- **[VERIFIED]** Kernel bug found and fixed during the phase: the gradient
  gather used the gradient-table base as the permutation base, which silently
  corrupted every layer after the first while layer 0 looked correct. The
  kernel now threads `perm_base` and `grad_base` separately.
- **[OPEN]** An explicit-`fma()` leaf variant paired with a `mul_add` CPU leaf
  reference was not added; the Phase 24 ladder already establishes that
  pairing (`R1` max absolute `2.3841858e-7` on RADV, 0 on llvmpipe/radeonsi),
  so re-running it for leaves would restate the settled contract rather than
  test the production reference.

## Phase 32 — Hybrid prototype (GPU separable leaves + CPU pointwise graph) — DONE; frozen (G32 not met)

- Files: `crates/horizon-client/examples/gpu_leaves_hybrid.rs` (342 lines,
  read-only example) and a new additive `DensityGraph::rebind` in
  `crates/horizon-core/src/density/graph/mod.rs` (`rebind(id, sampler, range,
  axes)` replaces a definition with a precompiled `Bound` sampler, keeping the
  function's identity; no behaviour change for existing callers). The
  wgpu dispatch is shared with the probe through
  `crates/horizon-client/examples/gpu_leaves_gpu.rs`. No production worldgen
  file is changed; the production injection seam (`lod_grid` in
  `worldgen/generator.rs`, `fill_noise_inner` in `worldgen/terrain.rs`) is
  off-limits by mission rule (both protected), so the prototype instead binds
  leaf samplers inside the density graph itself.
- **[VERIFIED]** Overworld `final_density` compiled with the far-LOD coarse
  lattice (`5 x 49 x 5` per chunk, images at `y = -64`, cell `4 x 8 x 4`),
  seed 0, RADV Vulkan, `shader_f64=true`:
  - `--chunks 16`: reference `105.62 ms` (single thread), hybrid graph
    `83.89 ms` + GPU leaf dispatch/readback `40.18 ms`; max difference in the
    final density field `2.593e-6`; **0 threshold flips of `density > 0` in
    19,600 coarse-cell samples**.
  - `--chunks 256`: reference `1616.23 ms`, hybrid graph `1391.22 ms` + GPU
    leaves `179.68 ms`; `6.31 -> 6.14 ms/chunk`; **speedup 1.03x**; max
    difference `3.010e-6`; **0 flips in 313,600 samples**.
- **[VERIFIED]** The hybrid rebinds only the 31 separable `minecraft:noise`
  leaves. `minecraft:old_blended_noise` (the single most expensive leaf,
  1.187 ms/chunk in Phase 30) and the two non-shifted `minecraft:blended_noise`
  sources are not GPU-backed, so roughly half the leaf evaluation cost stays on
  the CPU; this, plus ~0.2–0.4 ms/chunk of host-side table building and
  readback in `gpu_leaves_gpu.rs` (`prepare` builds Perlin tables per leaf,
  readback is mapped synchronously), is why the measured saving is about
  `0.88 ms/chunk` rather than the Phase 30 ablation figure.
- **[INFERRED]** Phase 30's ablation (leaves constant, graph drops 4.85 ->
  0.39 ms) is *not* a rebind ceiling: constant folding lets the pointwise
  graph slice and delete nodes, whereas GPU values are real data, so the
  pointwise graph keeps its full shape. The honest hybrid ceiling is the raw
  Perlin evaluation cost, ~30 % of the lattice cost (2.26 ms of 4.85 ms).
- **[VERIFIED]** G32 throughput (`>= 1.08x` proto throughput) is **not met**:
  1.03x at 256 chunks steady state, 0.85x at 16 chunks (GPU setup dominates).
  G32's flip count is met (0) and the prototype is numerically sound, but the
  freeze condition applies: **the hybrid GPU-density-leaves direction is
  frozen**. A production variant would need (a) GPU-resident `old_blended_noise`
  and `blended_noise`, and (b) device-side readback avoided by evaluating the
  pointwise graph on the GPU or feeding a GPU-built block mask downstream; (a)
  alone plausibly clears 1.08x. Neither is pursued without an explicit scope
  decision (A2).
- **[VERIFIED]** The `rebind` seam itself is clean: substituting GPU volumes
  through it produced a field that differs from the reference only in the low
  bits of the GPU leaf computation, with no sampler-cache interference (a
  pre-rebind CPU sampler cached by `get_sampler` would otherwise shadow the
  substitution; compiling the root only after all rebinds avoids that, and the
  hybrid's 2.593e-6 residual confirms the GPU leaves, not the cache, served
  the points).

## Phase 33 — Light stage split and GPU light contract — DONE (read-only; contract defined; no GPU work started)

- **[VERIFIED]** Measured with `gen_light_bench .local/horizon-client/26.3
  --radius 3 --threads 1` (meshing off, fresh layout): 49 columns in
  `2.52 s`, `51.5 ms` CPU per column; the light job (`streaming::
  job_stats` kind 4) was **49 x 1.23 ms = 0.06 s** — about 1.2 % of the
  column's CPU and ~0.02 % of wall time. Generation is `137 x 15.21 ms`.
  Far-LOD does not relight at all: the far-LOD column path samples density
  only and never calls `light_chunk`.
- **[VERIFIED]** There is one solver, not two. `light_chunk`
  (`crates/horizon-client-render/src/light.rs:316`) solves the whole 3x3
  region; the caller (`streaming.rs:1653`) picks the apply path afterwards:
  first lit -> `apply_lighting` (overwrite each section, share uniform
  sections); a relit chunk (a neighbour loaded or changed) ->
  `apply_relighting`, which writes only the sections whose cells changed and
  reports a dirty-section bitmask (`LightChanges`, `#270`). "Relight" in this
  codebase is the *same* solve plus a diff-aware apply — not a cheaper
  incremental solve — so a GPU port of the solver serves both paths, but the
  apply path's `LightChanges` comparison must be ported too if relight dirtiness is to
  match vanilla/Horizon remeshing.
- **[VERIFIED]** `light_chunk` internal stages (read-only analysis,
  `light.rs:316-570`): (1) section unpack + block-info scatter + emission
  seeds; (2) per-column top-down sky scan computing `lowest[x,z]`
  (`packed_edge_occluded` early break); (3) border seeding from
  `min(own, max neighbour lowest)`; (4) two queue-based BFS spreads (sky
  then block) with `block_masks` facing-tests, level decreasing by damp;
  the queue is capped at 65,536 entries; (5) `pack_lighting` copying the
  `h`-high volume into `sections * 4096` cells, plus a uniform-section
  fast path (`si * 16 >= h` -> `0xF0`); (6) three zeroed `h * W * W`-cell
  scratch arrays per call (~164 KB at h = 64), reused thread-locally.
- **[OPEN]** No bench separates these stages, and `gen_light_bench`
  exercises only the initial solve (every column is fresh there). A
  `LightChanges`-driven relight cannot be costed by that bench; it needs an
  edit workload. Splitting the stages would mean editing production code
  (out of scope for this read-only phase), so no stage-level millisecond
  figures are recorded. Stage (4) is the dominant term by inspection
  (queue BFS vs linear scans).
- **GPU whole-chunk light — contract sketch, feasibility only:**
  - *Inputs*: the 3x3 region's block-state ids (`48 x sections*16 x 48`),
    `sky_light` bool, `h` (top occupied section + 1, bounded above by
    `sections * 16`), and `LightTables` (per-id `info: u16`, per-type face
    masks, `block_masks`). Missing neighbour chunks are **opaque-dark**:
    `present[][]` gates both light holding and passing (`states[q]` of a
    missing column is neither written nor read), and the Nether case
    (`sky_light == false`) skips stage (2)/(3).
  - *Kernel*: grid over `W x h x W`; stage (1) scatter is embarrassingly
    parallel; stage (2) is 2,304 independent sequential scans (one lane per
    column); stage (4) as 15 fixed synchronous relaxation waves (level
    strictly decreases, damp >= 1, so the least fixpoint is unique and
    order-free — unlike density, nothing is approximate); stage (5) packs to
    the `sections * 4096` output layout byte-for-byte.
  - *Contract*: **integer-exact — no floating point anywhere**; a GPU run
    must produce the identical `Light` output as the CPU solver, so bit-exact
    parity is trivially testable (proptest against `light_chunk` on fixture
    chunks; the parity harness `light_parity` already pins the old
    algorithm). Must remain bit-exact for `h > sections * 16`? No: `h` is
    clamped by the CPU. Must replicate `present[][]`, emission seeding
    (stage 1 queues `emission > 0` before BFS), and the `si * 16 >= h`
    uniform-section rule. Must keep `LightChanges` class bitmask semantics for
    the relight path (bits 0-5 per 16-cell class via
    `LightChanges::class(x, y, z)`). Not required: identical queue visit
    order (invisible to the output).
  - *Cost shape*: ~110k cells, 15 waves, 6 neighbour tests per cell per
    wave per channel — well under 100 M cell-ops; host round-trip is
    dominated by upload/readback, not solve time. No resource-limit concern.
  - *Value*: negligible for the far-LOD density question (light is
    1.2 % of column CPU and absent from far LOD), moderate for load time on
    huge fresh worlds where 49 light jobs is not the bottleneck either;
    worth nothing unless a far-LOD GPU demo wants it.
- **Verdict:** the solver is GPU-friendly *and* integer-exact (no AMD/RADV
  caveat applies — unlike density), and the natural port unit is the whole
  solve with a `LightChanges`-producing pack step. It is **not pursued**:
  far-LOD density (this mission's target) never runs it, and its measured
  share of generation is too small to matter. Recorded so the decision is
  reproducible rather than forgotten.

## Phase 34 — Final report and verdict

### Mission recap

Decide whether a hybrid GPU density pipeline is viable for far-LOD terrain:
GPU evaluates interned separable noise leaves on the coarse lattice, CPU
finishes the pointwise graph. Exact vanilla `final_density` on GPU stays
frozen; all work is additive examples/probes; no production worldgen file was
touched.

### Phase table

| Phase | Scope | Result | Gate |
|---|---|---|---|
| 28 | Vanilla bundle staging; export coverage of density fields | done; 0/8 overworld fields exported, `final_density` 229 reachable / 120 supported / 109 blockers | — |
| 29 | Read-only: what far-LOD actually samples | done; far-LOD consumes `final_density`; per-chunk eval counts per level 0/2/4/6 = 1,225 / 14,161 / 7,225 / 3,757; density dominates | — |
| 30 | Leaf share of `sample_volume` | done; **~95 % by ablation**, 30 % by isolation, 37 post-interning leaves (32 separable); `old_blended_noise` biggest at 1.187 ms/chunk | **G30 PASS** |
| 31 | wgpu separable-leaf kernel (`gpu_leaves_probe.rs`) | done; RADV GPU-only 6.1–6.3k chunks/s, GPU+narrow 3.06k; radeonsi 4.40k GPU-only / 2.46k GPU+narrow; readback 148.3 KiB/chunk; 0 sign flips in 9.7M points; **llvmpipe 31/31 bit-exact**; RADV/radeonsi worst 3.755e-6 / 3.844e-6 (rel 1e-5) | **G31 PARTIAL** — throughput passes vs proto baseline; parity is a documented tolerance on AMD, not the 2.4e-7 contract (llvmpipe exact) |
| 32 | Hybrid prototype (`gpu_leaves_hybrid.rs` + `DensityGraph::rebind`) | done; 1.03x at 256 chunks steady state, 0.85x at 16; max field diff 3.0e-6; **0 threshold flips in 313,600 samples**; `old_blended_noise`/`blended_noise` still CPU-side | **G32 FAIL** (needs >= 1.08x) — direction frozen |
| 33 | Light split (`light.rs`) — read-only | done; light = 1.23 ms/chunk, 1.2 % of column CPU (radius 3, 1 worker, meshing off); one solver, two apply paths; GPU whole-chunk light contract sketched (integer-exact) | not pursued — far-LOD never lights |
| 34 | This report; `cargo test -p horizon-core -p horizon-world` (all ok), rustfmt, `git diff --check`, probe files all < 500 lines | done | — |

### Per-adapter coverage (all measured on the dev machine; WGPU/Naga 30.0.1)

| Adapter | Backend | fp64 | G31 parity | Notes |
|---|---|---|---|---|
| AMD Radeon Graphics (RADV REMBRANDT) | Vulkan | yes | worst 3.755e-6, rel 1.0e-5, 0 flips / 9,721,600 | not <= 2.4e-7; latency GPU-only 6.1–6.3k c/s |
| AMD Radeon Graphics (radeonsi, rembrandt, ACO) | GL | no | worst 3.844e-6, rel 1.0e-5, 0 flips / 9,721,600 | not <= 2.4e-7; 4.40k c/s |
| llvmpipe (LLVM 22.1.8, 256 bits) | GL | no | **31/31 leaves bit-exact, 0 mismatches (16 chunks)** | 381 c/s GPU-only (expected slow) |

### Files changed / added (nothing committed; `.local/` untouched by git)

- New examples: `gpu_leaves_probe.rs` (257 lines), `gpu_leaves_hybrid.rs`
  (342), `gpu_leaves_kernel.rs` (121), `gpu_leaves_tables.rs` (125),
  `gpu_leaves_gpu.rs` (268) — `crates/horizon-client{-lib}/`; registered in
  `crates/horizon-client-lib/Cargo.toml`.
- Additive API: `DensityGraph::rebind` (`crates/horizon-core/src/density/graph/mod.rs`).
- Read-only example: `crates/horizon-world/examples/hybrid_leaf_probe.rs` (Phase 30).
- Docs: `docs/gpu-noise-parity-progress.md` (Phases 28–34), plus pre-existing
  unrelated modifications already in the worktree from earlier phases
  (`docs/README.md`, `docs/api/shaders.md`, `docs/shaders.md`, density-IR
  design notes, vanilla_router_export, density probes).
- `DensityCompiler`/`router` files touched in Phase 28 (`compiler.rs`,
  `router.rs`, density `mod.rs`, `tests.rs`) are earlier phases' work, also
  uncommitted.

### Residuals and open items

- **[OPEN]** `old_blended_noise` (1.187 ms/chunk) and the two non-shifted
  `blended_noise` sources are not GPU-backed; rebinding them is the one
  plausible route to G32, but requires a GPU spline-blend kernel of the kind
  sketched in Phase 3 (WGSL spline+blend mismatch work) — out of scope under
  the freeze decision.
- **[OPEN]** `gpu_leaves_gpu.rs` rebuilds Perlin tables on every dispatch
  (`prepare` per call) and maps readback synchronously; a persistent table
  cache is required for any future production attempt.
- **[OPEN]** RADV/radeonsi do not meet the literal `<= 2.4e-7` contract for an
  ordinary-WGSL leaf; meeting it requires an explicit-`fma()` leaf paired with
  a `mul_add` CPU reference (Phase 24 pairing) — restate the contract, not the
  kernel, if ever needed.
- **[OPEN]** Relight (`apply_relighting`) path cannot currently be benched;
  `gen_light_bench` does the initial solve only.
- **[OPEN]** G31's written 2x gate uses the proto baseline (359 c/s); vs the
  probe's own 4-worker CPU leaf rate the margin is marginal (~6.1–6.3k vs
  ~6.6k needed). Recorded, not hidden.

### Verdict: **FREEZE** the hybrid GPU density-leaf direction

G30 passed on the merits of the ablation; G31's throughput gate passes as
written but its parity condition is met literally only on llvmpipe, with
documented, flip-free AMD tolerance; G32 fails (1.03x < 1.08x) even though the
field is numerically clean (0 threshold flips in 313,600 far-LOD samples
across 256 chunks). Exact vanilla `final_density` on GPU stays frozen. No
production wiring was or will be done without a separate scope decision
(A2) covering the protected `lod_grid`/`fill_noise_inner` seams, GPU-resident
blended leaves, and the contract restatement. Nothing is committed, pushed,
or posted; `.local/` remains untracked; all tagged numbers are either
[VERIFIED] (this machine, named backend), [INFERRED] (with stated basis), or
[OPEN] (with the missing evidence named).

## Phase 35 — Preserve the hybrid GPU track as patches — DONE

- **[VERIFIED]** The uncommitted hybrid/GPU-noise work is snapshotted as
  `docs/gpu-noise-parity/002-hybrid-density-new.patch` (24 new files,
  6,954 lines) and `docs/gpu-noise-parity/001-hybrid-density-modify.patch`
  (7 modified files, 586 lines), produced with `git diff` against `HEAD`
  (`eaa7da4fb`); no history rewrite, no stash, no reverts. The index was
  restored afterwards (`git reset -q`); the two staged files
  (`docs/api/shaders.md`, `docs/shaders.md`) were re-staged to their
  pre-snapshot state.
- **[VERIFIED]** Patches parse (`git apply --stat`): 001 = 7 files,
  487 insertions, 1 deletion; 002 = 24 files, 6,810 insertions. They capture
  Phases 26–34 as of this snapshot; later phases are not included.
- **[VERIFIED]** Default build unaffected: additions are examples only plus
  one additive method (`DensityGraph::rebind`); no production worldgen file
  was edited. `cargo check -p horizon-core -p horizon-world` emits no
  warning/error lines; `cargo test -p horizon-core -p horizon-world` passes
  (339 + suites green, same as Phase 34).
- **[INFERRED]** "Default build unchanged" in the strict sense (the default
  `cargo build` producing bit-identical artifacts) cannot be literally
  asserted from a clean checkout here — the default-profile game build was
  not relinked during this phase — but no default-target source input
  changed behaviourally: the diff contains only new example files, two
  additive methods/bindings (`rebind`, example registration) and docs.

## Phase 36 — Light cost reconciled, real worker pool, meshing on/off — DONE

- **[VERIFIED]** Machine (all Phase 36–39 numbers): AMD Ryzen 5 6600H
  (6 physical cores / 12 SMT threads, all online), Linux 7.2.0-1-cachyos-bore-lto,
  rustc 1.97.1, `fast` profile (release, codegen-units=16, no LTO, incremental).
  Load average from co-tenants swung between ~1.5 and ~5 during the session;
  every headline number below states its denominator, and noisy runs are
  called out rather than averaged away.
- **[VERIFIED]** `gen_light_bench` now has an additive `--mesh` flag
  (default off, unchanged), so the same bench covers both pipeline shapes;
  it reproduces identical chunk state in all six runs (checksum
  `8d759dfac1bf4f28` every time, including the two noisy 4-worker runs).
  Bench: `gen_light_bench <resources> --radius 5 --threads {1,4,8} [--mesh]`,
  113 columns lit, real shared worker pool (`HORIZON_JOB_THREADS`, pinned per
  child run), seed 1234, center (-60, 10). `mesh` job count is per finished
  column (~14: the column is remeshed for each of its neighbours finishing).
- **[VERIFIED]** Meshing OFF (numbers per lit column; shares use the
  `job_stats` per-kind sum as denominator, i.e. worker CPU excluding the
  coordinator; process-CPU/column includes coordinator and is printed by the
  bench):
  | threads | generate 233 jobs | decorate 169 | finish 113 | light 113 | other 62 | light share | process CPU/column |
  |---|---|---|---|---|---|---|---|
  | 1 | 233 x 11.72 ms = 2732 ms | 169 x 2.53 = 428 | 113 x 0.29 = 33 | 113 x 1.16 = 131 | 62 x 0.25 = 15 | **3.9 %** (0.131/3.34 s) | 30.6 ms |
  | 4 (two runs) | 233 x 24.38 = 5682 / 233 x 26.82 = 6250 | 169 x 4.93 = 833 / 169 x 5.56 = 940 | 113 x 0.52 = 59 / 113 x 0.63 = 71 | 113 x 2.04 = 231 / 113 x 2.50 = 283 | 63 x 0.42 = 26 / 61 x 0.51 = 31 | **3.4 % / 3.7 %** | 61.3 / 68.0 ms |
  | 8 | 233 x 14.74 = 3434 / 233 x 14.80 = 3450 | 169 x 3.07 = 519 / 169 x 3.02 = 510 | 113 x 0.36 = 41 / 113 x 0.35 = 40 | 113 x 1.59 = 180 (both runs) | 62 x 0.32 = 20 / 63 x 0.30 = 19 | **4.3 %** | 37.4 / 37.5 ms |
- **[VERIFIED]** Meshing ON (same 113 columns):
  | threads | jobs added (mesh) | mesh share | light share | process CPU/column | wall |
  |---|---|---|---|---|---|
  | 1 | 1632 x 0.08 = 131 | 3.6 % | 3.6 % (0.13/3.60 s) | 33.3 ms | 3.61 s |
  | 4 | 1656 x 0.08 = 132 | 3.7 % | 3.9 % (0.14/3.56 s) | 32.1 ms | 1.07 s |
  | 8 | 1631 x 0.10 = 163 | 3.7 % | 4.1 % (0.18/4.39 s) | 39.4 ms | 0.80 s |
- **[VERIFIED]** The 4-worker non-mesh run reproduces its inflation in both
  runs (~26 ms/chunk generate vs ~11.7 at 1 worker): the machine had a
  co-tenant burst; the checksum still matches, so the work is identical, just
  slower. Both values are reported instead of picking one.
- **[VERIFIED]** The Phase 33 "1.2 %" figure was an arithmetic slip: 1.23 ms
  of light vs 51.5 ms of process-CPU/column (radius 3, 1 worker, n = 49
  columns) is 2.4 %, not 1.2 %. The small-radius number is also a
  small-sample outlier; at radius 5 the light job is **3.4–4.3 %** of worker
  CPU (1.16–2.50 ms/lit column), or 3.6–4.1 % with meshing on. The "1.2 %"
  claim in the Phase 34 table is corrected here.
- **[OPEN]** The "~7–11 %" light share has no source anywhere in this repo
  (docs/, changelog, ledger search). What this repo can attribute to
  "lighting" today is a floor, not the 1.16 ms figure: the light job itself
  (4 %), `Job::LightView` (proto light-view snapshots, charged to `other`,
  0.015–0.03 s) and part of the coordinator's 3x3 light-region construction
  (coordinator 0.03–0.12 s CPU; not separable from meshing/decorate
  scheduling without instrumentation, out of scope for a read-only phase).
  Even generously charging the whole coordinator and all `other` jobs to
  light yields ~7 %, not "7–11", and overcounts. No repo evidence supports a
  higher light share; the 7–11 % figure is recorded as unverified external
  folklore (possibly from vanilla JVM whole-game profiles or from a
  radius-12 minefail-style measurement mixing relight). If a target is
  needed for load-time work, use the measured 3.4–4.3 % (worker CPU) /
  ~1 ms per lit column.
- **[INFERRED]** Why the measured share is stable across worker counts and
  meshing modes: light placement is on the critical path per column (it runs
  only after the 3x3 neighbourhood is decorated), so its cost is roughly
  additive per column and mostly CPU-bound (dense BFS over 48x16x48 cells,
  integer); meshing jobs steal per-column work too and dilute the mix
  rather than displace light.
- **[OPEN]** Stage-level split of `light_chunk` (emission scattering, sky
  seeding, BFS, pack) and of `apply_relighting` remain unmeasured; the
  light batch is opaque to `job_stats`. See Phase 33 for the split sketch.

## Phase 37 — Structures stage scaling, 1/4/8 threads — DONE (read-only)

- **[VERIFIED]** `chunkgen_bench <resources> --size 16 --threads T --warmup
  --no-map --json`, 256 proto chunks, seed 0, three runs per setting,
  `taskset -c 0-11`, machine as in Phase 36. Median rows (first runs in
  each triple discarded as warm-up noise — Rust book-keeping values
  reproduced to < 1 % once warm):
  | threads | wall | process CPU | ms/chunk/thread | biomes | noise | surface | carvers | **structures** |
  |---|---|---|---|---|---|---|---|---|
  | 1 | 2.66 s | 2.65 s | 10.4 | 0.82 | 3.47 | 3.97 | 1.12 | **1.04** |
  | 4 | 0.857 s | 2.78 s | 13.4 | 0.86 | 3.74 | 4.12 | 1.18 | **3.39** |
  | 8 | 0.596 s | 3.35 s | 18.6 | 1.03 | 4.53 | 5.08 | 1.31 | **6.44** |
  (The two noisy regime outliers this session — 4t with co-tenant burst:
  structures 7.5–7.8, all stages inflated; and the sibling-pin run below —
  are reported separately.)
- **[VERIFIED]** Structures wall-clock per chunk grows **1.04 → 3.39 →
  6.44 ms** (3.3x at 4 threads, 6.2x at 8). Over the same range every other
  stage grows <= 30 % (noise +30 %, surface +28 %, carvers +17 %, biomes
  +26 %). Process CPU per chunk grows only 10.4 → 10.9 → 13.1 ms (+26 %), and
  non-structures stages account for ~all of it:
  (4.53−3.47) + (5.08−3.97) + (1.03−0.82) + (1.31−1.12) = **2.57 ms/chunk at
  8 threads**, leaving roughly zero CPU for the structures wall growth.
- **[VERIFIED]** Worker-wall minus process CPU per chunk (i.e. time workers
  were not on a core): 1 thread ≈ 0.0 ms, 4 threads ≈ **+2.5 ms**, 8 threads
  ≈ **+5.5 ms** — matching the structures growth (+2.35 / +5.40 ms) almost
  exactly. At 4 and 8 threads, effectively **100 % of the structures-stage
  growth is blocked time**, not computation.
- **[VERIFIED]** Preemption/SMT contention exists but is uniform across all
  stages, not structures-specific: pinning 4 threads to two SMT sibling
  pairs (cores 0,1 and 2,3) inflates *every* stage ~1.5x (structures 3.6–3.9,
  noise 5.74–5.79, surface 6.72–6.74, CPU 4.17–4.19 s); pinning to four
  distinct physical cores (0,2,4,6) in a quiet machine reproduces the 1-extra-
  core relationship of the default runs. SMT accounts for the ~x1.3 on noise/
  surface at 8 threads (12 SMT threads on 6 physical cores), and for zero of
  the structures excess.
- **[VERIFIED]** The stage timer (`StageTimings`, `generator.rs:2355, //` 
  `structures`) is wall-clock (`lap(&mut clock)` with `Instant::now()`,
  `generator.rs:2341`), so wait time lands in it. The measured signature —
  per-chunk wall inflation with flat process CPU — is the classic
  lock-convoy shape, not duplicated work.
- **[VERIFIED]** Where the wait comes from, by code inspection:
  `references()` (`generator_structures.rs:696`) scans a 17x17 neighbourhood
  per chunk and calls `build_starts(chunk, wait = false)`; on a WouldBlock
  race the caller falls back to `starts_at()` which **blocks on the
  per-chunk `start_work` mutex** (`generator_structures.rs:513-520`,
  `starts_at` line 405). The 8 workers generating adjacent chunks all wait
  on the same in-flight neighbour builds; `StartCache` is sharded
  RwLock (`StartCache`, line 147) with a read-first fast path, so its cost
  is cache traffic, not the convoy. `create_starts` itself
  (climate quarts + set placement) happens once per start and is parallel;
  waiting, not creating, is what scales with worker count.
  Also, `references()` goes through `StructureQueries`, whose `structure_biomes`
  cache is **per-worker** (`WorkerState.structure_biomes`,
  `generator.rs:2369`), so overlapping neighbourhoods are re-sampled per
  worker — duplicated CPU work exists but is bounded by the ~0.1–0.2 ms/chunk
  per-worker growth, an order of magnitude below the wait.
- **[VERIFIED]** Repeated-run medians, same flags, same seed: structures
  1.01–1.04 (1 t), 3.35–3.41 (4 t), 6.41–6.45 (8 t) — < 2 % spread. The
  earlier one-shot numbers in `opencode/structures_scaling_note.md`
  (1.02 / 3.21 / 6.81) reproduce.
- **[INFERRED]** The `worker.timings.structures` wall inflation at 8 threads
  (6.44 ms/chunk = ~35 % of worker-side wall at that setting) costs about the
  same wall time as the whole noise stage (4.53 ms). Fixing the convoy, not
  the placement math, is the lever.
- **Classification: ~90–100 % lock-convoy wait** (`start_work` per-chunk
  mutex + waiting `starts_at` after a WouldBlock race in `references()`);
  small irreducible duplicated CPU (per-worker biome-quart caches); SMT/core
  oversubscription affects all stages uniformly (~x1.3 at 8 threads) and
  explains none of the structures excess.
- **Candidate fixes, with estimated impact** (read-only phase — none of these
  is implemented):
  1. **Break the block in `references()`**: treat a WouldBlock neighbour as
     "undecided" for this pass and seed beardifier/reference checks
     conservatively (vanilla loads far starts lazily at feature placement,
     not eagerly at terrain fill). Impact: removes the ~5.4 ms/chunk of
     structures wall at 8 threads (~25–30 % of proto wall). Risk:
     underestimating a far-side start's reach in deviations from vanilla;
     needs a second pass once all neighbours are decorated (already the
     pipeline's decorate boundary). Complexity: medium.
  2. **Defer structure references to the decorate stage** (or compute them on
     the coordinator after the 3x3 neighbourhood is ready): removes the wait
     entirely from proto generation. Impact: similar to 1, plus shrinks the
     proto critical path; the beardifier fields then bind only when all
     neighbour starts are known, which is when they are first *used*.
     Complexity: high (order-of-driver change, `Pipeline` fields).
  3. **Raise `start_work` granularity**: per (chunk, structure set) keys
     instead of per chunk, or per (chunk, ring-set): lets independent sets
     proceed while one builds. Impact: partial (removes set-vs-set convoy,
     keeps chunk-vs-neighbour wait; ~20–40 % of the structures excess
     estimated). Complexity: medium.
  4. **Share the `structure_biomes` quart cache across workers** (bounded
     LRU/shard): cuts the duplicated climate samples. Impact: ~0.1–0.2 ms/chunk
     CPU (~2–4 % of structures stage at 1 t, less at high t). Complexity: low.
  5. **Leave one core for the main thread / reserve SMT siblings**
     (`HORIZON_JOB_THREADS` policy): the ~x1.3 SMT inflation on all stages at
     8 threads on 6 cores; no effect on the convoy itself. Complexity: trivial
     (pool sizing policy), benefit everywhere except the structures stage's
     wait component.
- **Issue/PR check (repo docs only)**: `ROADMAP.md` lines 37 and 163 list
  #1760 and #1778 as **in review** (#1760 "free structure starts in the
  generation workers", #1778 part of the same lock-convoy work; both listed
  under "Steady frames at long render distances"). No changelog file in the
  repo mentions them; `docs/known-issues.md` does not either. The measurement
  above matches the roadmap's own framing ("lock convoy on structure starts").
- **Draft comment — DO NOT POST** (drafted for #1760 / #1778; for the
  maintainer to adapt):

  > Follow-up on the structure-start convoy from the #1760 work: on
  > Ryzen 5 6600H (6c/12T), Linux 7.2, Horizon @ fast profile, seed 0,
  > `--size 16 --warmup --no-map --json`, medians of 3:
  > structures stage wall per proto chunk 1.04 / 3.39 / 6.44 ms at 1 / 4 / 8
  > threads, while process CPU per chunk grows only 10.4 → 10.9 → 13.1 ms and
  > worker-wall minus CPU is ~0 / +2.5 / +5.5 ms/chunk — i.e. ~all of the
  > growth is blocked time in the `start_work` per-chunk mutex (via
  > `references()` → `build_starts(wait=false)` → fallback `starts_at`), not
  > duplicated work. Per-worker `structure_biomes` caches duplicate climate
  > samples but account for at most ~0.1–0.2 ms/chunk of CPU. Possible
  > directions: break the block in `references()` (lazily resolve references
  > at decorate), defer references to after the 3x3 neighbourhood is ready,
  > or per-(chunk, set) start-work keys. Classification and full table are
  > in `docs/gpu-noise-parity-progress.md`, Phase 37.

- **[OPEN]** The lock-owner wait is inferred from the wall/CPU signature and
  the code path, not from a mutex-hold profile (no `perf`/`valgrind` on this
  host; `eu-stack` cannot unwind the release build). If a future
  `perf`-equipped host confirms the signature, the classification here needs
  no change in substance.

## Phase 38 — Surface stage breakdown (per column vs per block) — DONE (read-only)

- **[VERIFIED]** Measurement tool: new example
  `crates/horizon-world/examples/surface_stage_probe.rs` (~330 lines). It
  rebuilds the real surface worker for the overworld's `minecraft:overworld`
  material rule from the `.local` bundle (same rule library, same compiled
  rule, same density sources, same per-column traversal and counter
  evolution), times each public bucket, and never writes to the chunk.
  Biome queries are answered uniformly with "minecraft:plains"; the rule
  tree evaluation is real (plains-biome estimate, see caveat below). No
  protected file is edited; default builds do not build examples.
- **[VERIFIED]** Seed 0, `--chunks 2` and `--chunks 4` runs are consistent
  (try_apply per call 0.14 us both runs). Per proto chunk (~32.9k cells,
  ~128 y-steps per column, ~31.6k stone-ish cells, ~31.6k rule calls):
  | bucket | ms/chunk | share | us/call |
  |---|---|---|---|
  | column setup: top scan (WorldSurfaceWg heightmap) + x/z gradients + `biome_at` + `update_xz` | 0.018–0.049 | ~0.3 % | 0.07–0.19 us per column |
  | descend-loop cells: `block_properties` + counter maintenance | 1.46–3.63 | **~20 %** | ~0.05–0.1 us per cell |
  | `update_y` (above/below counters) | 1.16–2.84 | **~16 %** | — |
  | `try_apply` (full rule tree) | 4.33–11.38 | **~62 %** | 0.14–0.36 us per stone cell |
  | total (probe buckets) | ~7.0–17.9 | — | — |
- **[VERIFIED]** Machine state dominates absolute numbers this session
  (co-tenants): the probe total and the bench's `surface` stage timer move
  together (same regime, ~7.2 ms probe vs 9.3 ms bench surface timer in the
  noisy regime; probe buckets sum to ~77–80 % of the stage timer, the
  remainder being `set_block` writes, `WorldSurfaceWg` priming, and the
  badlands/frozen-ocean extensions). Shares are regime-stable.
- **[INFERRED]** Plains-uniform biome queries understate the rule tree's
  cost: every wooded-badlands/dripstone/frozen-peaks biome branch resolves
  false. Overworld chunks with mixed biomes pay more per `try_apply`.
  Rank-order within the executed set is representative; absolute try_apply
  cost in production is likely **higher** by an unknown margin (no per-biome
  split was feasible read-only).
- **[VERIFIED]** Top-3 hot buckets (all measured with `Instant` inside the
  example; "hot functions" = public functions the real stage also calls):
  1. `MaterialConditions::try_apply` — 62 % of surface-stage CPU;
     dominated inside by `noise_threshold` conditions against
     `minecraft:surface` (43 occurrences across 27 rules) and `y_above`
     (23 occurrences), i.e. named-noise sampling, one cached 2D sample per
     (column, revision).
  2. Descend-loop cell handling (`block_properties` + `is_air`/`has_fluid`
     branch) — ~20 %; pure traversal, scales with filled blocks.
  3. `update_y` counter maintenance — ~16 %; per stone cell, called before
     the rule.
- **[VERIFIED]** Per-column vs per-block split: **per-block work is ~99.7 %**
  (31.6k rule-evaluated cells/chunk at ~0.14–0.36 us each); **per-column work
  is ~0.3 %** (256 columns at ~0.07–0.19 us). The stage is block-bound; a
  GPU/vector rewrite of the traversal helps the ~20 % bucket most, but the
  62 % rule bucket is the target — and its cost is bounded by noise sampling,
  which Phases 30–32 froze.
- **[INFERRED]** If only the rule were free, the surface stage would drop
  from ~4–9 ms to ~2.7–6.5 ms/chunk; if the traversal were free too, ~2.6
  more ms would disappear. There is no cheap exact fix on this path that
  does not touch attempt-order or caching discipline of the rule runner
  (e.g., deciding `above_preliminary_surface` once per column instead of per
  stone cell, or hoisting uniform-biome gradient checks) — such edits must
  respect vanilla evaluation order and are out of scope here.
- **[OPEN]** Same measurement for a biome-mixed region (e.g. `--chunks 16`
  at a badlands-heavy seed), and a band-splitting of `try_apply` itself
  (above/below surface, stone_depth ranges) needs either a perf-capable host
  or edits to material-condition internals; both excluded from this phase.

## Phase 39 — Final re-baseline and recommended order

Machine: AMD Ryzen 5 6600H (6c/12T), Linux 7.2.0-1-cachyos-bore-lto, rustc
1.97.1, `fast` profile (release, codegen-units=16). All numbers tagged; all
walls from `--warmup` medians; `structures` and `surface` columns are medians
of 3 runs; the machine's co-tenant noise is documented per row. ms/chunk =
per proto chunk at that worker count.

| Stage | 1 t | 4 t | 8 t | Ceiling if free | Platform independence | Exactness risk |
|---|---|---|---|---|---|---|
| biomes | 0.82 | 0.86 | 1.03 | ~8% of proto CPU | portable (same data path everywhere) | parity-critical (biome ids feed rules) |
| noise (fill+aquifer) | 3.47 | 3.74 | 4.53 | ~33% | portable code, arch-sensitive floats (x86 FMA vs ARM) | **parity-critical** — Phase 34 verdict: frozen, no GPU, no reordering |
| surface | 3.98 | 4.12 | 5.08 | ~38% | same as noise | evaluation-order risk if hoisted — not attempted |
| carvers | 1.13 | 1.18 | 1.31 | ~11% | portable | carving-mask parity harnesses pin it |
| structures | 1.04 | 3.39 | 6.44 | 10% @1t → **35% @8t** | **algorithmic — portable by construction** (mutex convoy) | none with lazy references; order-of-attempts must be preserved |
| decorate (3x3 region) | 2.53 | 4.93 (single noisy run) | 3.07 (single noisy run) | ~25% of job CPU | portable; parallel already | feature-placement order pinned by design tests |
| finish | 0.29 | 0.52 | 0.36 | ~2% | portable | none known |
| light (initial solve) | 1.16 | 2.04 | 1.59 | ~1.4% of process CPU; 3.4–4.3% of worker CPU | **integer-exact — fully portable** | none (bit-exact) |
| mesh | ~1.16 ms per loaded column (14 jobs x 0.08–0.10 ms) | | | ~110 ms per 100 columns | portable | none (visual artifact only) |

### Reading the table

- **At 1 thread**: surface (3.98) + noise (3.47) dominate proto CPU
  (~10.4 ms/chunk); structures is cheap (1.04).
- **At 8 threads**: structures (6.44) is ~35 % of worker-side wall and costs
  more than the whole noise stage — and it is **~100 % blocked time**
  (wall/CPU signature; Phase 37). Noise/surface grow only ~x1.3 from
  SMT/oversubscription.
- **Contrary to the earlier far-LOD hypothesis, the biggest actionable cost
  in the shipping path is a scheduling problem, not arithmetic.**

### Recommended order (all read-only input for future phases)

1. **Structures-start convoy** (#1760/#1778, in review): break the block in
   `references()` / defer references until the 3x3 neighbourhood is decorated
   (or per-(chunk, set) `start_work` keys; see Phase 37 candidate fixes 1–3).
   Impact: ~2.3–5.4 ms/chunk wall at 4–8 workers (~15–30 % of proto wall).
   Exactness risk: none if attempts/order are preserved; parity harnesses for
   structure placement already exist (`structure` probes / `#1760`
   reproducer).
2. **Worker-pool sizing vs SMT**: leave SMT siblings out of worldgen or cap
   gen workers at physical cores minus one for light/mesh; ~x1.3 on noise and
   surface at 8 threads on this 6c/12T machine. Exactness risk: none (pool
   sizing, not math).
3. **Surface stage**: hoist `above_preliminary_surface` and
   `minimum_surface_level()` to once-per-column (small reserved deletions,
   ~0.5–1 ms/chunk plausibly); bigger wins need rule-runner caching work
   behind parity gates. Exactness risk: moderate; attempt order must stay.
4. **Noise**: do not touch — frozen (Phase 32/34).
5. **Light**: measure, then ignore — integer-exact, 3.4–4.3 % of worker CPU;
   relight split remains [OPEN] (no bench exists).
6. **Decoration order**: already parallel by phase; no action.

### Verdict on the GPU density track (unchanged)

The GPU/noise track stays **frozen**: G30 pass (ablation), G31 partial
(throughput pass vs proto baseline; llvmpipe bit-exact; AMD documented
tolerance, no sign flips but no ≤2.4e-7), G32 fail (1.03x < 1.08x). Nothing
in Phases 35–39 changes that verdict. The repo's actionable generation
headroom is (1) the structures convoy and (2) SMT-oversubscription polish —
algorithms and pool policy, not kernels.

### Constraint check for this phase

- `cargo test -p horizon-core -p horizon-world` — pass (re-run for Phase 39).
- Default build warning-free: Phases 35–39 add only examples and docs;
  `cargo check -p horizon-core -p horizon-world` is clean.
- Every probe file < 500 lines (`surface_stage_probe.rs` 303; `gen_light_bench.rs` ~410).
- Every number above is tagged [VERIFIED] (this machine, named flags),
  [INFERRED] (basis stated) or [OPEN] (missing evidence named).
- `.local/` untouched by git; no commits, pushes, or posts (#1760/#1778
  comment only drafted, not posted); no history rewrite; protected files
  (`noise/perlin.rs`, `density/interpolation.rs`, `worldgen/generator.rs`,
  `worldgen/terrain.rs`) unedited.

## Phase 40a — In-review scope check (#1760, #1778) — DONE (read-only)

- **[VERIFIED]** Repo documents that name #1760/#1778: `ROADMAP.md` line 37
  ("free structure starts in the generation workers (#1760)", listed under
  "Steady frames at long render distances", in review) and line 163 ("Cut the
  per-column generation cost (noise sampling, surface rules, aquifers) and
  the lock convoy on structure starts, and leave one core free for the rest
  of the game as vanilla does (#1760 and #1778, in review)"). No other doc
  names them; `docs/known-issues.md`, changelog (absent) and the parity
  progress file before Phase 40 do not. No offline PR body/review text is
  present in this clone.
- **[VERIFIED]** Consequence for candidate selection: the two highest-impact
  candidate fixes — (1) break the wait in `references()` (deferral of
  far-side references to the decorate stage, when all neighbours are
  decorated and therefore cached) and (2) reshuffling referenced at the
  pipeline level — are (i) output-ordering changes that are exactly the
  #1760/#1778-shaped work, and (ii) the same waiting issue those PRs are
  presumably resolving. Per Phase 40a's instruction they were dropped.
- **[INFERRED]** #1778 ("leave one core free") is a scheduling-policy concern
  (pool sizing), not an algorithmic one; it maps to Phase 41, not Phase 40b.
- **[VERIFIED]** The remaining candidates were reduced to: (a) eliminate the
  blocking wait in `references()` by deriving missing starts independently
  (bit-identical output, duplicate work for host chunks); (b) narrow the
  per-chunk `start_work` mutex for positions no set can host (empty fast
  path); (c) share the per-worker climate-biome caches. (a) was measured and
  rejected (see Phase 40b); (b) was implemented and measured; (c) is
  sub-noise (~0.1–0.2 ms/chunk of CPU, below the run-to-run co-tenant
  variance) and was not implemented.

## Phase 40b — Structures-stage fix attempt — DONE; G40c NOT MET

### Where the wait actually is (instrumented, temporary, reverted)

- **[VERIFIED]** Temporary debug counters (`structures_probe`,
  env-independent atomics; removed before the final build) accumulated
  per-process totals across 256 proto chunks, seed 0, `--size 16
  --warmup --no-map --no-decorate`, median-equivalent single runs:
  | segment | 1 thread | 8 threads |
  |---|---|---|
  | `references()` total | 347 ms | 1,612–1,629 ms |
  | `…` build loop (`build_starts`, wait=false) | ~27 ms | ~374 ms |
  | `…` blocking fallback in the `starts_at` arm | — | **1,254 ms (32 waits, ~39 ms each)** |
  - rows from distinct runs; absolute values bounce with machine load, the
  ratios do not.
- **[VERIFIED]** The wait sits in the `starts_at` fallback, not in
  `build_starts`: when another worker is building a structure-hosting chunk
  (this area contains exactly one host, the mineshaft at (12,-13)), every
  scanning worker that reaches the position in the same window gets `None`
  from `try_lock` (`WouldBlock`) and then parks on the same mutex. 32 waits
  x ~39 ms = ~1.25 s — the structures-stage excess at 8 threads.
- **[VERIFIED]** The host builds happen ~16 ms each (slow arm: 683 ms at 1
  thread, 360 ms at 8) — jigsaw decode/assembly is the serial hot region.
- **[VERIFIED]** The two plausible bit-identical alternatives both fail:
  1. *Empty fast path* (share a static empty `Starts` for positions no set
     can host; skip the per-chunk work lock and `try_lock` traffic):
     implemented as the Phase 40 fix. Output bit-identical (G40a passes on
     12/12 snapshots). Measured effect on structures-stage wall: **zero
     within noise** (see G40c) — the empty-position traffic was never the
     convoy; the wait is happens-before on one live build.
  2. *Independent derivation as a wait-free fallback* (in the blocking arm:
     derive the same starts locally, deterministic, insert first — the same
     `insert_first` semantics as today): measured **slower** —
     `references()` 1.61 s → 2.54 s at 8 threads, 34 duplicate mineshaft
     assemblies ≈ +2 s CPU. Rejected; reverted.

### What shipped (the fix)

- `crates/horizon-world/src/worldgen/generator_structures.rs`: in
  `build_starts`, after the cache check and the old-chunk check, a new fast
  rejection returns the shared static `EMPTY_STARTS` when **no**
  `possible_sets` entry's `is_structure_chunk` passes for this chunk (rings
  resolved first, exactly as the locked path does). Identical output to the
  locked path (all-false means `create_starts` yields an empty vector, which
  is exactly what the locked path returns and caches), identical placement
  order (empty), ~35 lines, no protected file touched.
- `crates/horizon-client/examples/chunkgen_bench.rs`: adds an opt-in
  `CHUNKGEN_NO_STRUCTURES=1` diagnostic that builds the generator without
  the structures registry (for the before/after attribution to the
  structures stage only; not a shipping configuration).

### Gate results

- **G40a PASS** — `chunkgen_bench --size 16 --warmup --no-map
  --no-decorate --snapshot`, seeds **0 and 1234**, threads **1, 4, 8**; the
  `-terrain.bin` and `-decorated.bin` SHA-256 are **identical before and
  after** the fix (12/12; before was produced by `git checkout` of the file
  and rebuilt; after re-applied via `git apply`). Before-fix snapshots were
  also bit-identical across 1/4/8 threads, confirming the harness's
  determinism as a baseline.
- **G40b BLOCKED** — `python3 tools/parity.py chunk_generation` (and every
  `structure_*` harness) cannot run in this clone: the vanilla oracle
  artifacts are absent (`.local/minecraft/26.3/manifest.json` does not
  exist; the harness fails inside `verify_artifacts` identically on HEAD
  and with the fix; `.local/parity-runner` holds a recorded fail from this
  attempt). `cargo test -p horizon-world --lib`: **339 passed** with
  `--test-threads=2` (the default parallelism SIGKILLed the test process
  under co-tenant memory pressure both times — not a test result);
  `cargo test -p horizon-core`: 10 suites ok. These do not substitute for
  the vanilla harness; they are recorded as the offline floor of evidence.
- **G40c FAIL** — structures-stage wall per proto chunk (`chunkgen_bench
  --size 16 --warmup --no-map`, medians of 3, `taskset -c 0-11`):
  | threads | before fix | after fix |
  |---|---|---|
  | 1 | 1.01 / 1.04 | 0.99 / 1.01 / 2.42(warm-up r1, same machine) |
  | 4 | 3.35 / 3.39 / 3.41 | 3.15 / 3.20 / 3.58 |
  | 8 | 6.41 / 6.44 / 6.45 | 5.90 / 6.23 / 6.28 |
  | **ratio 8t/1t** | **6.2x** | **5.9–6.2x** |
  chunks/s (same runs): before 97/299/430; after 96/306/439. The gate asks
  for ≤ 2x; the untouchable part of the wait (happens-before on the host
  build) is what remains. No gate criterion is fudged or re-measured away;
  all runs are reported.
- **G40d PASS** — `cargo test -p horizon-core -p horizon-world` passes
  (horizon-world with `--test-threads=2`, see above); default builds
  warning-free (the fix adds one static and one branch, no public API
  change); rustfmt clean; `git diff --check` silent; no protected file
  edited; probe counters reverted.

### Root-cause verdict and the honest boundary

- **[VERIFIED]** The structures-stage wait is an intrinsic happens-before on
  the in-flight build of the one structure-hosting chunk in the measured
  area (the mineshaft at (12,-13), start chunk box `[110,-41,-282]..
  [256,-17,-113]`), not lock inefficiency, not duplicated derivation, not
  SMT or allocator pressure. Every chunk whose 17x17 neighbourhood contains
  the host park-waits on it through `starts_at`'s `work.lock()`.
- **[VERIFIED]** Any in-place, bit-identical, smaller-than-restructure fix
  is exhausted by the two measured attempts. Moving the wait requires the
  pipeline restructure (defer structure references until the 3x3 region is
  decorated, or build far-host starts ahead of the chunks that scan them),
  which is #1760/#1778-shaped work (in review, per ROADMAP.md).
- **Status: A2 (user-scope decision) — the continuation that could meet
  G40c is out of this phase's mandate and out of review scope.** Not
  blocked on resources; blocked on ownership of the pipeline change.
- The shipped fast path stays (bit-identical, harmless, smaller than any
  alternative in reach); it is labelled as an attributive clean-up, not a
  fix.

## Phase 41 — SMT-aware pool sizing (read-only) — DONE

- **[VERIFIED]** In this repo the worker count for the worldgen/mesh pool is
  decided by (in order): `options.txt` `horizon.workers` (graphics setting,
  UI "Auto (N)", 0 = Auto, capped at 64) → `horizon_client_render::graphics::
  Settings::horizon.workers`; Auto computes `horizon_threads::worker_count()
  - 1` = `HORIZON_WORKER_THREADS`-or-`available_parallelism()` minus one
  (11 on this 6c/12T box); on battery halves it (ceil 11/2 = 6).
  `horizon_threads::jobs::default_threads()` repeats the same rule for the
  shared `jobs` pool; `Pool::new` spawns `horizon-worker-N`, niced on Linux
  (`behind_server`). `HORIZON_JOB_THREADS` additionally overrides at pool
  init (benches only). The streaming/far pools all derive from the same
  count (far pool a third of it).
- **[VERIFIED]** A/B on this 6c/12T machine, `chunkgen_bench --size 16
  --warmup --no-map` (medians of 2–3, `taskset`):
  | pins | interpretation | wall | cpu | structures ms/chunk |
  |---|---|---|---|---|
  | `0-5` | 3 physical cores via SMT pairs | 0.86–0.87 s | 4.10–4.12 s | 4.96–5.03 |
  | `0,2,4,6,8,10` | 6 distinct physical cores, no SMT | 0.636 s | 2.82 s | 4.79 |
  | `0-11` (default) | all 12 SMT threads | 0.58–0.61 s | 3.26–3.35 s | 5.90–6.28 |
  - SMT siblings cost ~x1.4 on every stage versus the same physical cores
    alone; 6 threads on 6 physical cores matches the 12-thread default and
    costs less CPU. The 8-thread solution used by `--threads 8` is already
    inside the SMT tax (~x1.3 on noise/surface vs 4 threads).
- **[INFERRED]** The Auto default (machine-threads minus one = 11 here) is
  acceptable: the pool is niced behind the render/server threads and they
  yield on Linux; workers mostly park when the game is idle. The measured
  SMT tax shows up in worldgen throughput, but deciding that the worldgen
  pool should cap at physical cores would need an end-to-end frame-rate
  bench (`tools/graphics_bench.py`), not proto-bench throughput. #1778's
  framing ("leave one core free as vanilla does") is already the behaviour.
- **Recommendation (no code change)**:
  1. Keep `HORIZON_WORKER_THREADS` (documented as the escape hatch in this
     phase) and `horizon.workers` in options.txt.
  2. Do NOT add SMT detection to `worker_count()` now; rebaser/pkg regression
     risk on non-x86 and on machines where SMT helps; measure with
     `tools/graphics_bench.py` first (6 vs 11 Auto on a fresh world load).
  3. If the maintainer wants it: `auto = (physical_cores) - 1` where
     physical cores come from `/sys/devices/system/cpu/cpu*/topology/
     thread_siblings_list` (Linux) with a fallback to
     `available_parallelism() - 1` elsewhere — small, but belongs to #1778's
     scope, not this phase's.
- **[OPEN]** No frame-rate or tick timing data was collected for the 6 vs
  11 default; the proto-throughput table above is the only evidence.

## Phase 42 — Surface `try_apply` hoisting plan (read-only) — DONE

- **[VERIFIED]** The surface-stage buckets from Phase 38 (plains-uniform,
  seed 0): `try_apply` ≈ 62 % of surface CPU, descend-cell bookkeeping
  ≈ 20 %, `update_y` ≈ 16 %, column setup ≈ 0.3 %. `try_apply` runs
  ~31.6k times per chunk at ~0.14–0.36 us per call.
- **[VERIFIED]** Which conditions in the overworld rule tree are
  column-invariant, and their current caching:
  | condition | inputs | column-invariant? | cached already? |
  |---|---|---|---|
  | `minecraft:biome` | (x,z) quart | yes | yes — `Context::biome` memo per block revision; the underlying `Biomes::Region` query is quart-cached |
  | `minecraft:noise_threshold` with `is_3d: false` | 2D noise at y=0 | yes | yes — `CachedNoise` keyed on `column_revision` |
  | `minecraft:temperature` | biome + sea level | yes | yes — column revision, `cold_enough_to_snow` with worker temperature caches |
  | `minecraft:steep` | column geometry | yes | yes — `steep`/`hole` are column-revision predicates |
  | `minecraft:y_above` | y, anchors | no | n/a — y-dependent |
  | `minecraft:stone_depth` | stone-above counter | no | n/a — `update_y` maintains it |
  | `minecraft:water` | stone_depth + offsets | no | n/a |
  | `minecraft:above_preliminary_surface` | y vs minimum | no | partially — `minimum_surface_level()` memoized per column; the `y >= min_surface` compare itself is per stone cell by design |
  | `minecraft:gradient` (`Gradient{low,high,random}`) | y, RNG | no | no — 3D positional RNG per cell |
  - `noise_threshold` thresholds are compared against cached noise values;
    the per-call residue is two float comparisons and a revision check.
- **[INFERRED]** Because the column-invariant predicates are already
  memoized per column (or are O(1) per cell), the obvious "hoist the
  column-invariant checks out of the y loop" buys **≈ 0–2 % of the surface
  stage**, i.e. well under the run-to-run machine noise here, and it is not
  free: `rule(y, above)` is dispatched per cell by `surface_variants`'s
  band windows, so any specialization must preserve the band dispatch and
  the exact attempt order or the parity harnesses would fail.
- **[INFERRED]** The biggest *sizable* win would instead be avoiding rule
  evaluation for stone cells far below the surface — the overworld rule can
  place blocks only near the surface/frontier and in the ore-vein bands;
  proving that no rule can fire below a y threshold (without changing any
  result bit) needs the rule-tree's static target-depth analysis (the
  `Gradient` bands, all `y_above anchors`, all `stone_depth` offsets) and a
  parity harness run to prove output identity. That is the only hoist with
  real payoff (~40-60 % of the 31.6k calls, ≈ 20-35 % of the stage) and it
  is *not* a hoist: it's a skip rule with a static proof obligación, needs a
  harness run to certify, and is protected-file-scale work.
- **Guarding harness**: `chunk_generation` parity (currently BLOCKED in this
  clone, Phase 40b) + `material_rule_specialization.rs` /
  `material_stage.rs` Rust tests + `surface_stage_probe` numbers +
  the mod_content material stage tests. Any hoist **must** keep all of them
  green with identical block output.
- **Recommendation**: do not hoist anything now. `noise_threshold`/`y_above`
  per-column memoization already exists; attempting anything else is a
  parity-risky change with ≤ ~2 % upside. Revisit only with the vanilla
  oracle available for the certification run.

## Phase 43 — Final report (phases 40–42) and PR-ready draft (DO NOT POST)

### Phase table

| Phase | Result | Gates |
|---|---|---|
| 40a | In-review scope checked (ROADMAP.md only offline): #1760/#1778 are the deferral-pipeline and pool-policy work; picked non-overlapping candidate | — |
| 40b | Structures fix attempt: empty-position fast path **shipped**; wait-free duplication rejected by measurement; convoy proven to be happens-before on one live host build | G40a **PASS** (12/12 snapshots identical), G40b **BLOCKED** (oracle artifacts absent in this clone), G40c **FAIL** (5.9–6.2x, unchanged), G40d **PASS** |
| 41 | SMT-aware pool sizing assessed; 6 physical cores beat 3 SMT pairs by x1.4 | recommendation only, no code |
| 42 | Surface hoisting plan: no free wins — column-invariant predicates already memoized; the one big win is a skip rule needing a static proof and the (blocked) vanilla oracle | recommendation only |

### Before/after, structures stage (ms/chunk, medians of 3, `taskset -c 0-11`, seed 0)

| threads | structures before | after fix | wall before | after fix | chunks/s before/after |
|---|---|---|---|---|---|
| 1 | 1.01 / 1.04 / 1.02 | 0.99 / 1.01 / 2.42* | 2.64–2.69 s | 2.64 s | ~97 / ~96 |
| 4 | 3.35 / 3.39 / 3.41 | 3.15 / 3.20 / 3.58 | 0.857 s | 0.83–0.86 s | ~299 / ~306 |
| 8 | 6.41 / 6.44 / 6.45 | 5.90 / 6.23 / 6.28 | 0.596 s | 0.58–0.59 s | ~430 / ~439 |

\* first run hit the same co-tenant burst the baseline runs hit; it is reported, not discarded.

### Files changed/added this mission (all uncommitted)

- Modified: `crates/horizon-world/src/worldgen/generator_structures.rs` (fast
  rejection in `build_starts` + static `EMPTY_STARTS`),
  `crates/horizon-world/examples/chunkgen_bench.rs` (opt-in
  `CHUNKGEN_NO_STRUCTURES=1` diagnostic), `crates/horizon-client/examples/gen_light_bench.rs`
  and Phases 36–39 docs/probes from the previous mission.
- Appended docs: Phases 40–43 sections above.
- Untouched/protected: `noise/perlin.rs`, `density/interpolation.rs`,
  `worldgen/generator.rs`, `worldgen/terrain.rs`.

### PR-ready draft — DO NOT POST (no PR exists or is intended; author is the maintainer)

> **Title (draft):** worldgen: return the shared empty starts for
> structure-hostless chunks without claiming the per-chunk work lock
> > **🤖 Opened by <login>'s AI agent (<tool, model>), not by <login> personally.**
>
> `references()` probes a 17x17 neighbourhood per chunk. For the vast
> majority of those positions no structure set can host anything here
> (`is_structure_chunk` is false for every set on this seed), but the
> serialized build path still claims the per-chunk `start_work` mutex and
> runs an empty build. This change hands those probes the shared empty
> vector before the lock, using the same predicate the build uses.
>
> Measured on the dev machine (Ryzen 5 6600H, 6c/12T, seed 0): generated
> proto chunks are bit-identical to before the change across seeds 0 and
> 1234 at 1/4/8 workers (terrain and decorated snapshots, SHA-256).
> Structures-stage wall time is unchanged within noise (it is dominated by
> the happens-before wait for the one in-flight structure host in the
> tested area). This is an attribution/clarity clean-up, not a throughput
> fix; throughput work remains with #1760/#1778.
>
> 🤖 Generated with [<tool>](<tool-url>)

**Verdict: do not merge as a "fix" and do not post; this is a marker of how
far the safe local fix reaches.** The remaining convoy needs the
#1760-style pipeline change (defer reference resolution past the 3x3
neighbourhood) or the vanilla-parity oracle restored in this clone to even
certify a restructure.

### Residuals

- **[OPEN]** `.local/minecraft/26.3` artifacts absent — every vanilla
  parity harness is un-runnable in this clone (chunk_generation,
  structure_*). Restore `python3 tools/lab.py prepare` companion data from
  a machine with a real Minecraft install to revive the gates.
- **[OPEN]** The 32-wait convoy needs the #1760/#1778 pipeline fix; out of
  scope here; duplicating the jigsaw assembly (wait-free derivation) is
  bit-identical but measured slower end-to-end.
- **[OPEN]** Empty fast path kept: bit-identical, but moves no measurable
  wall time; keep or drop at the maintainer's discretion.
- **[VERIFIED]** `cargo test -p horizon-world --lib` only passes with
  `--test-threads=2` on this machine under co-tenant memory pressure
  (default parallelism SIGKILLed the test binary; it is not a test
  failure — see Phase 40b/G40d).

## Phase 44 — R1-R6 as hand-written SPIR-V through passthrough — DONE (G44 PENDING-HW)

Question: with naga out of the path, can the R1-R6 ladder be stated *as the
SPIR-V contract itself* and come out bit-exact? naga's WGSL output carries no
`NoContraction` at all (Phase 44 inspection: 0 decorations, no
`FPFastMathMode`, 13 `OpExtInst Fma` from `mul_add`, 14 `OpFMul`), so a WGSL
probe can never tell a driver *not* to contract. This phase removes both the
translator and that ambiguity.

### What was built

- `crates/horizon-client/examples/spv/perlin_ladder.spvasm` — the whole
  ladder (R1, R2 at alpha 0/0.375/1, R3, R4, R5, R6) in one compute module,
  414 lines, all eight outputs per sample. Every float
  add/sub/mul/div carries `NoContraction` (34 decorations); every site where
  the Rust reference calls `mul_add` is an explicit `GLSL.std.450 Fma`
  (12 of them). No fast-math mode anywhere.
- `crates/horizon-client/examples/perlin_spirv_probe.rs` — assembles it with
  `spirv-as --target-env vulkan1.3` at run time and submits it through
  `device.create_shader_module_passthrough` (`Features::PASSTHROUGH_SHADERS`).
  Also checks that the decoration count equals the arithmetic count before
  dispatch, so a silently dropped `NoContraction` fails the probe.
- `tools/perlin_spirv_probe.sh` — one command: prints the adapter/driver, the
  `VkPhysicalDeviceFloatControlsProperties` for f32, whether
  `VK_KHR_shader_float_controls2` is present, then the per-rung mismatch table
  for three module variants:

  | Variant | Header | Meaning |
  |---|---|---|
  | `base` | none | `NoContraction` + explicit Fma only |
  | `--float-controls` | `DenormPreserve` + `SignedZeroInfNanPreserve` | adds SPV_KHR_float_controls2 execution modes |
  | `--allow-contraction` | none, decorations stripped | negative control: is the driver already exact? |

- `--float-controls` needs `VK_KHR_shader_float_controls2`; without it shader
  module creation fails and the probe says so instead of dropping the request.

### Result on llvmpipe (software Vulkan), not hardware

- Adapter **[VERIFIED]**: `llvmpipe (LLVM 22.1.8, 256 bits)`, Mesa 26.2.5,
  Vulkan 1.4.354, `deviceType=CPU`, `driverID=DRIVER_ID_MESA_LLVMPIPE`,
  `VK_KHR_shader_float_controls2` present, f32 properties
  `denormBehaviorIndependence=ALL`, `roundingModeIndependence=ALL`,
  `shaderDenormPreserveFloat32=false`, `shaderSignedZeroInfNanPreserveFloat32=true`,
  `shaderRoundingModeRTEFloat32=true`, `shaderRoundingModeRTZFloat32=false`.
  Run under the flatpak Mesa 26.2.5 lavapipe ICD (system lavapipe wants
  `libLLVM.so.23.1`, absent).
- **[VERIFIED]** All three variants read 0/60,000 on all eight rungs, bit
  for bit, including the 30,000,000-scale coordinates, exact integers and the
  `±16,777,216` clamp points in the sample set:

  | Rung | base | float-controls | allow-contraction |
  |---|---:|---:|---:|
  | R1 leaf | 0/60,000 | 0/60,000 | 0/60,000 |
  | R2 alpha 0 | 0/60,000 | 0/60,000 | 0/60,000 |
  | R2 alpha 0.375 | 0/60,000 | 0/60,000 | 0/60,000 |
  | R2 alpha 1 | 0/60,000 | 0/60,000 | 0/60,000 |
  | R3 spline | 0/60,000 | 0/60,000 | 0/60,000 |
  | R4 coordinate offset | 0/60,000 | 0/60,000 | 0/60,000 |
  | R5 add/mul/blend | 0/60,000 | 0/60,000 | 0/60,000 |
  | R6 router root | 0/60,000 | 0/60,000 | 0/60,000 |

- **[VERIFIED] The negative control also passes, so this run does not show
  that the contract is what saves us on llvmpipe.** LLVM on x86-64 contracts
  mul+add only under fast-math `contract`, which an SPIR-V module without
  `NoContraction` does not request. The decorations are therefore *load
  bearing on a GPU that contracts by default* (RADV), and inert on a CPU
  backend. That is exactly the case G44 has to settle on hardware.
- **[VERIFIED]** Two assembly bugs were found by the probe and fixed, both
  logic, not driver: the final trilinear lattice had the x and y fade
  swapped (R1/R3/R5 wrong everywhere), and R4's y shift was
  `off.y - (-0.25)` instead of `off.y + (-0.25)`. Before the fixes the
  constant rungs (R2 alpha 1, R6) were already 0/60,000, which localised the
  fault to the Perlin leaf rather than to loading, bindings or readback.
- **[VERIFIED]** The same input/output plumbing passes with a WGSL shader in
  the same example (`--wgsl-control`), so a passthrough failure would be
  about the module, not the buffers.
- **[VERIFIED]** `spirv-val --target-env vulkan1.3` accepts the base module;
  naga's `spv-in`/`wgsl-out` round-trip reads back as the intended program
  (checked independently of the driver), which is how the fade-order bug was
  confirmed.

### G44 — PENDING-HW

G44 (RADV reads 0/60,000 on R1-R6 with the contract present) **cannot be
decided in this sandbox**: every run above is llvmpipe. The hardware command
is unchanged and one line:

```sh
tools/perlin_spirv_probe.sh
```

and the report is complete when the `base` table is 0/60,000 on all rungs and
the header shows the RADV adapter. The `--allow-contraction` table is the
interesting second number: if RADV diverges there and not under `base`, the
contract is sufficient and a passthrough leaf tier is *permitted*; if it
diverges under both, a GPU density tier cannot be exact on RADV at all.

### Residuals

- **[OPEN]** G44 needs the hardware run; no claim about RADV is made here.
- **[OPEN]** llvmpipe reports `shaderDenormPreserveFloat32=false` yet accepted
  and executed the `DenormPreserve 32` execution mode without a validation
  error. Treat the float-controls rung as "requested and accepted", not as
  "denormal behaviour proven" on this device.
- **[VERIFIED]** `cargo test -p horizon-core -p horizon-world` passes
  (126 core, 339 world plus the smaller targets).
- **[OPEN]** `cargo clippy -D warnings` on this toolchain fails in
  `crates/horizon-world/src/worldgen/aquifer/noise_based.rs:476`
  (`manual_range_contains`), a file untouched by this phase; the example
  itself is clean. Pre-existing on rustc 1.97.1.

## Phase 45 — Ceiling ablation for a GPU density tier — DONE (measured shares)

Question: even if the Perlin density graph ran on the GPU for free, how much
proto throughput could that buy? This is the upper bound any exactness work
has to beat, so it is measured before designing a tier.

Method and machine: `fast` profile, seed 1234, `chunkgen_bench --size 16`
(256 proto chunks, decoration on, no `--warmup`), `gen_light_bench
--radius 8 --repeat 1`; per-stage numbers come from the benches' own
`StageTimings`, so nothing here is profiled or inferred from code reading.
The density cost in the same session comes from `hybrid_leaf_probe
--settings minecraft:overworld --seed 1234 --chunks 64`.

### chunkgen_bench (proto, 256 chunks)

| Threads | Wall | chunks/s | ms/chunk | biomes | noise | surface | structures | carvers |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 4.50 s | 56.9 | 17.6 | 0.82 | 6.00 | 6.66 | 2.51 | 1.44 |
| 2 | 1.34 s | 191.7 | 10.4 | 0.42 | 3.23 | 3.47 | 2.46 | 0.76 |
| 4 | 0.791 s | 323.5 | 12.4 | 0.44 | 3.32 | 3.56 | 4.17 | 0.77 |
| 8 | 0.603 s | 424.5 | 18.8 | 0.54 | 4.30 | 4.64 | 8.23 | 0.91 |

- **[VERIFIED]** The `noise` stage (density fill + aquifer setup) is 6.00 ms of
  17.6 ms at one thread (34 %), 3.23/10.4 (31 %), 3.32/12.4 (27 %),
  4.30/18.8 (23 %).
- **[VERIFIED]** Same-session density cost: `final_density` on the coarse
  lattice is **5.93 ms/chunk** (block volume 7.42 ms), i.e. the noise stage is
  essentially the density graph on the interpolated lattice; the aquifer and
  heightmap remainder is under 0.1 ms at one thread.
- **[VERIFIED]** `hybrid_leaf_probe` this session: 37 unique leaves, 5.93 ms
  coarse lattice, leaves alone 2.32 ms (31 %), graph with all leaves constant
  0.40 ms (5 %). Same picture as Phase 30, so the leaf-share gate still holds.
- **[VERIFIED]** Process CPU does **not** scale: 4.46 s (1 t) → 2.36 s (2 t) →
  2.41 s (4 t) → 3.02 s (8 t) for the same 256 chunks. At 4 and 8 threads the
  run is contention-bound, so higher-thread ceilings should be read as "what
  this bench can show", not as machine throughput.

### Ceilings (Ahmdal over the measured stage table)

| Threads | Now chunks/s | noise free | speed-up | density leaf free | speed-up |
|---:|---:|---:|---:|---:|---:|
| 1 | 56.9 | 86.4 | 1.52x | 79.7 | 1.40x |
| 2 | 191.7 | 278 | 1.45x | — | — |
| 4 | 323.5 | 442 | 1.37x | — | — |
| 8 | 424.5 | 550 | 1.30x | — | — |

- **[INFERRED]** Making the whole noise stage free is the absolute ceiling for
  any GPU density work: **1.30x–1.52x** proto throughput, never more, because
  surface (6.66 ms) and the rest are untouched.
- **[INFERRED]** A GPU tier that costs anything at all lands below that. If
  half the CPU density time reappears as GPU-plus-transfer time (2.96 ms), the
  one-thread ceiling is 14.6 ms/chunk, 68.5 chunks/s, **1.20x**.
- **[INFERRED]** At 8 threads the tier is worth even less (1.30x free, ~1.15x
  at half cost) because structures/contention dominate the wall.

### gen_light_bench (radius 8, 253 lit columns)

| Workers | Wall | columns/s | generate | decorate | light | ms CPU/column |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 6.35 s | 39.8 | 5.10 s | 0.82 s | 0.30 s | 26.3 |
| 2 | 3.13 s | 80.7 | 4.85 s | 0.81 s | 0.31 s | 24.8 |
| 4 | 1.74 s | 145.6 | 4.94 s | 0.81 s | 0.32 s | 24.9 |
| 8 | 1.22 s | 207.7 | 6.19 s | 0.98 s | 0.42 s | 30.9 |

- **[VERIFIED]** Light is 1.18–1.66 ms of 24.8–30.9 ms CPU per column
  (4.5–5.4 %), which confirms the earlier "GPU light is not worth it" call.
- **[INFERRED]** The bench generates 421 chunks for 253 columns, so the density
  graph is about 5.93 × 421/253 = 9.9 ms per column (~38 % of column CPU). A
  free density stage would raise one worker from 39.8 to about 64 columns/s
  (1.6x). The ratio is similar at higher worker counts, but `generate` inflates
  there as well, so the absolute numbers move.

### What this settles

- **[VERIFIED]** The achievable ceiling is real but modest and **not the
  bottleneck at high thread counts**: at 8 threads the structures convoy costs
  more wall than the entire noise stage (8.23 vs 4.30 ms/chunk here). A GPU
  density tier cannot be sold as a throughput fix on this machine; the
  ordering of the earlier recommendation (scheduling first) still holds.
- **[OPEN]** A *measured* replay ablation (replace density/biome/aquifer with
  cached results and re-time) needs a switch inside
  `crates/horizon-world/src/worldgen/generator.rs`, which is protected by the
  mission. The ceilings above are therefore Amdahl over measured stage
  timings, not a replay run; the arithmetic is small enough that a replay would
  only confirm the shares.

## Phase 46 — Exact-or-off tier design — DONE (design only, no code)

The question a passthrough tier has to answer is not "can the GPU compute
Perlin" (R1–R6 say yes on a conforming device) but "when may it be enabled".
Design, read-only:

1. **Granularity is the device, not the chunk.** The tier is enabled per
   `(adapter, driver, contract)` triple and only when the ladder reads
   0/60,000 on every rung it uses. It is never enabled partially: a device that
   passes R1–R5 but not R6 runs all density on the CPU.
2. **Default off.** No auto-detection. The switch follows the Horizon-only
   feature rule: off unless the player turns it on, and a settings screen that
   names the adapter it was certified on.
3. **Contract is part of the certification.** The probe's three variants are
   the contract: `base` (NoContraction + explicit Fma) is the tier's required
   contract; `--float-controls` is optional and only used if the device also
   honours it; `--allow-contraction` is the diagnostic that says whether the
   contract was doing any work.
4. **Exact or CPU.** There is no tolerance mode. Output must be bit-identical
   to `evaluate_fma`, which is what the production density path computes.
5. **Certification is per release.** Driver updates can change fusion
   behaviour, so the probe is re-run on the release's target driver and the
   result recorded; a stale certificate means off.

### The 109 `final_density` blockers, by rung

Phase 28's export probe (still the count of record): 229 reachable nodes, 120
supported, 109 blockers. Mapping them onto the ladder shows how far R1–R6
actually reach:

| Family | Blockers | Covered by | Gap |
|---|---:|---|---|
| `UnsupportedOperation` — `Unary` 15, `Binary` 9, `Choice` 7, `Gradient` 6, `Clamp` 4, `IntervalSelect` 3, `Spline` 3 | 47 | R3 (Spline), R5 (Add/Multiply/Blend), R2 (lerp/Choice shape) | no rung for `Gradient`, `Clamp`, `IntervalSelect`; `Unary` only partially (no rung for the full unary set) |
| `NoiseKind` — `Noise` 34, `OffsetNoise` 2, `BlendedNoise` 1 | 37 | R1 (seeded Perlin), R4 (coordinate offset) | R1 takes one CPU-narrowed cell+offset per sample; production `Noise` carries `xz_scale`/`y_scale`, so a per-leaf scaled sample model is still missing, and `BlendedNoise` (old blended noise) has no rung |
| `NeedsContext` — `Cache` 16, `Interpolated` 5, `WorldField` 3, `BlendDensity` 1 | 25 | none | `Cache` is an identity unwrap at a fixed position but is unproven; `Interpolated` is the lattice the CPU already computes (R1 consumes its output); `WorldField`/`BlendDensity` have no rung |

- **[VERIFIED]** No single family makes any router field export, and
  `final_density` needs all three (Phase 28). So a tier that only covers what
  R1–R6 validate cannot be enabled for real chunks today: the blocker count
  that gates the switch is **109, not 0**.
- **[INFERRED]** The shortest path to 0 is `NoiseKind` + the identity `Cache`
  rule (climate fields reach 5/8), but `final_density` additionally needs the
  three unstated operation families (`Gradient`, `Clamp`, `IntervalSelect`) and
  their own rungs. Until each has a 0/60,000 rung on the certified device, the
  tier stays off.
- **[OPEN]** `BlendedNoise` is the largest single leaf (1.23 ms/chunk) and has
  no rung; it is the highest-value next ladder entry if the tier is pursued.
- **[OPEN]** Far-LOD consumes `final_density` too (Phase 29/30), so the same
  109 blockers gate that track as well.

## Phase 47 — Docs and final report — DONE

### Docs

- `docs/gpu-noise-parity.md` gained a "SPIR-V passthrough ladder" section: how
  to run `tools/perlin_spirv_probe.sh`, what the three variants mean, and that
  the ladder is the per-device certificate for the exact-or-off design above.
- This file: Phases 44–47 appended (Phase 44 kept **PENDING-HW** for G44).
- No source file under `crates/` changed for the design phases; the only code
  added is the probe example, the assembly text and the wrapper script.

### Final report

- **G44 is not answered.** Nobody ran the ladder on RADV. The hardware step is
  `tools/perlin_spirv_probe.sh`, and the answer is the `base` table: 0/60,000
  on all eight rungs means a passthrough density tier is *permitted*; any
  nonzero rung means it is forbidden on that adapter.
- **What the software run does establish**: the hand-written module is correct
  on a conforming Vulkan driver, the ladder harness is sound (bugs found were
  assembly logic, and the constant rungs isolated them), and the pipeline can
  express the exact contract end to end. llvmpipe passes with the contract and
  without it, so the decorations are inert on a CPU backend; only the RADV run
  can show they matter.
- **What the ceiling says**: even a free density stage is worth 1.30x–1.52x on
  proto and roughly 1.6x on lit columns, and less at 8 threads where
  structures dominate. A GPU tier is a load-time improvement, not a fix for
  the high-thread bottleneck.
- **What blocks a shipping tier**: 109 `final_density` blockers across three
  families; R1–R6 cover the leaf arithmetic and several composition shapes but
  not `Gradient`, `Clamp`, `IntervalSelect`, `BlendedNoise`, or the per-leaf
  scaled sample model.
- **[VERIFIED]** Gates: `cargo test -p horizon-core -p horizon-world` passes
  (126 core, 339 world, plus the smaller targets); `cargo fmt --check` clean
  for the touched crate. `cargo clippy -D warnings` fails on an untouched file
  under rustc 1.97.1 (`aquifer/noise_based.rs:476`, `manual_range_contains`);
  the new example is clean.
- **[OPEN]** `.local/minecraft/26.3` is still absent, so the vanilla parity
  harnesses remain un-runnable here (unchanged from Phase 40b).

## Phase 48 — Exact-or-off tier, rung by rung — IN PROGRESS (handoff)

Work stopped mid-rung 2. This section is the resume anchor; nothing below is
claimed as a passed gate unless it says VERIFIED.

### Rung 1 — `Cache` unwrap — DONE and gated

- **[VERIFIED]** `density/cache.rs` was read first: the scalar cache stores one
  packed `BlockPos` and returns it only for the same position; the volume path
  copies an exact-volume buffer. `Cache` is therefore pure memoization and its
  child can be exported in its place. `Interpolated` is *not* unwrapped: it
  changes the sampled lattice.
- `DensityCompiler::try_export_to_program` unwraps `GraphNode::Cache(input)` to
  `input`; `export_node_failure` marks `Cache` supported. Graph tests now assert
  the same bits at five sample points and that `export_blockers(blocked).len()`
  drops from 2 to 1.
- **[VERIFIED]** `cargo test -p horizon-core --lib`: 126 passed, 0 failed,
  3 ignored.
- **[VERIFIED]** `vanilla_router_export .local/horizon-client/26.3/worldgen
  --summary` on `minecraft:overworld`: `final_density` reachable 229,
  supported 136 (was 120), blockers 93 (was 109), first blocker
  `Unary (UnsupportedOperation)`. Still **0 of 8** fields export; the other
  seven fields are unchanged (`Noise (NoiseKind)` or
  `Interpolated (NeedsContext)`).

### Rung 2 — `Clamp`, `Unary`, `Binary` — CODE IN PLACE, GATE NOT RUN

- `DensityProgram` gained `UnaryKind` (`Abs`, `Negate`, `Square`, `Cube`,
  `HalfNegative`, `QuarterNegative`, `Reciprocal`, `Sign`, `Squeeze`),
  `BinaryKind` (`Add`, `Subtract`, `Multiply`, `Divide`, `Minimum`,
  `Maximum`), and the `Clamp`/`Unary`/`Binary` variants. `Sqrt` and `Log` stay
  out: they are `f64` libm calls whose `f32` rounding a GPU cannot reproduce.
- `evaluate` and `evaluate_fma` mirror `arithmetic.rs` (`java_min`, `java_max`,
  `clamp`, `UnaryOp::apply`, `BinaryOp::apply`); those three helpers are now
  crate-visible to `program.rs`.
- The graph exporter maps `Clamp`, every `Unary` except `Sqrt`/`Log`, and every
  `Binary`; `export_node_failure` agrees with it.
- **[VERIFIED]** `cargo test -p horizon-core --lib`: 126 passed. Nothing in the
  new path is exercised yet, so this only proves the crate compiles and the
  existing behaviour is intact.
- **New emitter**: `crates/horizon-core/src/density/spirv.rs` emits SPIR-V text
  from the IR instead of hand-extending `perlin_ladder.spvasm`. Every
  `OpFAdd`/`OpFSub`/`OpFMul`/`OpFDiv` carries `OpDecorate ... NoContraction`,
  and every `mul_add` site is an explicit `GLSL.std.450 Fma`. Float constants
  are written as exact bit patterns (`!0xXXXXXXXX`), which `spirv-as` /
  `spirv-val --target-env vulkan1.3` accept.
- **Not done**: the emitted module has not been assembled and dispatched from
  the tree, there is no rung-2 probe example yet, and the 0/60,000 llvmpipe
  gate has not run. The RADV script for rung 2 does not exist. Treat rung 2 as
  **PENDING-HW / unfinished**, not passed.
- **Known edge (document in the tier, not a bug in the mirror)**: the generic
  `java_min`/`java_max` semantics in the IR differ from the compiled scalar
  sampler's early-outs only for NaN operands and signed zero on the
  early-out path (multiply/divide by a zero left operand, min/max bound hit).

### Handoff notes

- Base commit `eaa7da4fb`. Upstream `origin/main` had advanced by 31,478
  commits when this was written; a rebase or a fresh port will be needed.
- No push permission on `horizon-rust/horizon` for the account used here and no
  fork exists, so the WIP branch is local only.
- Resume from `docs/session-handoff.md`; the next concrete step is to finish the
  rung-2 probe (assemble, dispatch, compare 60,000 samples against
  `evaluate_fma`) and then re-run `vanilla_router_export`.
