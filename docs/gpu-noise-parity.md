# GPU noise parity probe

> **These probes are not in this checkout.** `density_program_probe`,
> `wgpu_perlin_probe` and `tools/perlin_spirv_probe.sh` live in
> [#19144](https://github.com/horizon-rust/horizon/pull/19144), and
> [#19146](https://github.com/horizon-rust/horizon/issues/19146) is the task
> that ports the rung-2 part to `main`. The commands below become runnable only
> once those files and their example registrations exist here.

Run the WGSL comparison with:

```sh
LIBGL_ALWAYS_SOFTWARE=1 WGPU_ADAPTER_NAME=llvmpipe cargo run -p horizon-client-lib --example wgpu_perlin_probe
LIBGL_ALWAYS_SOFTWARE=1 WGPU_ADAPTER_NAME=llvmpipe cargo run -p horizon-client-lib --example density_program_probe
```

The probe uses a deterministic set of 60,000 coordinates. Rust computes the
lattice cells and f32 offsets from the Perlin table's f64 offsets; the WGSL
kernel performs permutation lookup, gradient dots, fade and interpolation. It
compares output bits with `PerlinNoise::get3`, then repeats the comparison for
`SmearedPerlinNoise::get3`. The acceptance rule for sampled values is bit
equality (zero ULP). This checks WGSL arithmetic given CPU-precomputed lattice
coordinates; it does not test GPU coordinate-to-lattice conversion. A passing
run applies only to the adapter printed by the probe; run it on each backend
that needs coverage.

This is a Perlin leaf check, not a comparison of the full world-generation
density graph. Separate interpolation and synthetic graph probes are described
below. Production NoiseRouter and terrain coverage remain open.

## Locked density-program arithmetic contract

For the manually built R1–R6 `DensityProgram` ladder, use explicit FMA: the
CPU reference uses `f32::mul_add` and WGSL uses `fma()` for fused gradient,
fade, interpolation, spline and blend operations. Other operations retain
their stated order. On llvmpipe (LLVM 22.1.8, 256 bits, Cpu, GL), WGPU/Naga
30.0.1, `shader_f64=false`, default pipeline options, the ladder was bit-exact
for 60,000 samples. On AMD Radeon Graphics (RADV REMBRANDT, IntegratedGpu,
Vulkan), WGPU/Naga 30.0.1, `shader_f64=true`, default options, its maximum
absolute error was `2.3841858e-7`. On AMD Radeon Graphics (radeonsi, Rembrandt,
ACO, Other, GL), WGPU/Naga 30.0.1, `shader_f64=false`, default options, it was
bit-exact for the same sample count. These are per-adapter observations, not
cross-backend guarantees. `RADV_DEBUG=noopt` did not improve RADV R1.

The separate `sqrt` diagnostic feeds arbitrary f32 bit patterns to WGSL `sqrt`
and compares them with `(value as f64).sqrt() as f32`. It is not part of the
Perlin acceptance rule. In particular, NaN payloads and subnormal handling can
produce bit differences even when ordinary positive inputs agree. WGSL defines
the `sqrt` domain as nonnegative values, permits subnormal flushing for
accuracy-listed operations, and gives f32 `inverseSqrt` a 2 ULP bound; `sqrt`
inherits its accuracy from `1.0 / inverseSqrt(x)`. See the [WGSL floating-point
rules](https://gpuweb.github.io/gpuweb/wgsl/#floating-point-evaluation).
For a future call site that actually uses sqrt, compare finite nonnegative
normal inputs within 2 ULP, treat subnormals under an explicit FTZ policy, and
do not use negative inputs as a conformance test.

## SPIR-V passthrough ladder (per-device certificate)

The WGSL probes above go through naga, whose SPIR-V output contains no
`NoContraction` decoration at all, so a WGSL comparison cannot ask a driver not
to contract or reassociate. The passthrough probe removes the translator:

```sh
tools/perlin_spirv_probe.sh
```

It assembles `crates/horizon-client/examples/spv/perlin_ladder.spvasm` with
`spirv-as`, submits it through
`device.create_shader_module_passthrough` (`Features::PASSTHROUGH_SHADERS`,
Vulkan only), and prints the adapter, driver, the f32
`VkPhysicalDeviceFloatControlsProperties`, whether
`VK_KHR_shader_float_controls2` is present, and the mismatch count for each of
R1-R6. The wrapper runs three variants:

| Variant | Contract |
|---|---|
| `base` | every float add/sub/mul/div `NoContraction`, every `mul_add` site an explicit `GLSL.std.450 Fma` |
| `--float-controls` | the above plus `DenormPreserve`/`SignedZeroInfNanPreserve` (`SPV_KHR_float_controls2`); fails loudly if the device or driver cannot take it |
| `--allow-contraction` | the base module with the decorations stripped: a diagnostic, not a tier |

On llvmpipe (Mesa 26.2.5, LLVM 22.1.8, Vulkan 1.4.354) all three variants read
0/60,000 on all eight rungs. The negative control also passing means the
decorations are inert on a CPU backend: only a run on a driver that contracts
by default (RADV) shows whether the contract is doing the work. A device is
"certified" for the exact-or-off tier described in
[docs/gpu-noise-parity-progress.md](gpu-noise-parity-progress.md) (Phase 46)
only when the `base` table is 0/60,000 on every rung.

## Additional probes

Run the narrower stages independently:

```sh
cargo run -p horizon-client-lib --example wgpu_lattice_probe
cargo run -p horizon-client-lib --example wgpu_density_interp_probe
cargo run -p horizon-client-lib --example wgpu_density_graph_probe
cargo run -p horizon-client-lib --example wgpu_sqrt_probe
cargo run -p horizon-client-lib --example wgpu_adapter_inventory
```

The lattice probe checks f32 coordinate stages and the permutation/gradient
indices. The interpolation probe checks scalar trilinear ordering and density
blend. The graph probe compares a compiled spline and blend over CPU-generated
Perlin leaves using the bulk `sample_volume` reference. It is not the complete
NoiseRouter or a chunk terrain run. It exercises the explicit-FMA contract
described above; strict-expression comparisons remain available in
`radv_divergence_probe` as a separate diagnostic. Both matched exactly on
llvmpipe for the probe samples.
WGSL does not expose a contraction-disable control; its ordinary expression
variant is measured on the backend, not guaranteed unfused. WGSL permits expression
fusion and permits `fma()` to expand to multiply-plus-add, so explicit FMA is
only verified on the tested backend and does not promise identical behavior on
every backend. Sample 66 demonstrates the contracts differ by one bit: strict
spline `0x3edd5a8e`, explicit-FMA spline `0x3edd5a8d`.

The sqrt probe is independent of Perlin parity. The adapter inventory reports
every adapter visible to wgpu on the current machine. All bit-exact results
are specific to the tested adapter and sample set.

## Density-program IR ladder

`horizon_core::density::program::DensityProgram` is an additive prototype. Its
CPU evaluator and WGSL code generator are exercised by `density_program_probe`.
`DensityCompiler::export_to_program` now exports a limited definition subset;
the IR is not used by production world generation. The detailed graph inventory,
representability decisions and extension plan are in
[density IR design](density-ir-design.md).

All ladder results below are **VERIFIED** on adapter `llvmpipe (LLVM 22.1.8,
256 bits) (Cpu, Gl)`, WGPU/Naga 30.0.1, `shader_f64=false`, with default
pipeline compilation options. The contract is bit-exact f32 equality for the
60,000 deterministic samples, with the Perlin leaf's f64 offsets and lattice
coordinates narrowed on CPU. Explicit spline FMA uses Rust `f32::mul_add`
paired with WGSL `fma()`. GPU codegen materializes the Perlin leaf in a prior
dispatch before composing R2/R3/R5, so compilation does not fuse leaf arithmetic
into a parent expression.

| Rung | Implemented subset | Mismatches |
|---|---|---:|
| R1 | Seeded Perlin table and narrowed lattice sample | **VERIFIED:** 0/60,000 |
| R2 | Constant plus lerp at alpha 0, 0.375 and 1 | **VERIFIED:** 0/60,000 each |
| R3 | One cubic spline segment over a Perlin leaf | **VERIFIED:** 0/60,000 |
| R4 | f32 coordinate offset of the Perlin sample | **VERIFIED:** 0/60,000 |
| R5 | Add, multiply and blend composition | **VERIFIED:** 0/60,000 |
| R6 | Custom trivial router `final_density = 0.125 + 0.125` | **VERIFIED:** 0/60,000 |

Phase 17 used a fixed 16×16×16 volume at chunk (0,0), seed 0. The custom test
router was compiled through the existing `NoiseRouter`/`DensityCompiler`. The
`fill_noise` API calls `fill_noise_inner`, whose recorded final-density buffer
was compared with a direct production sampler volume, the IR CPU evaluator and
the IR GPU output. Results were **VERIFIED**: (a) vs (b) 0/4,096, (b) vs IR CPU
0/4,096, and (b) vs (c) IR GPU 0/4,096. The positive density selected the
default solid block for all 4,096 voxels. This is a custom constant-arithmetic
test router, not the vanilla overworld preset and not full NoiseRouter
coverage.

Each probe accepts `WGPU_ADAPTER_NAME` as a case-insensitive adapter-name
substring and uses wgpu's environment-aware adapter selection. With the
variable unset, the probe requests the default adapter. On this run, setting
`WGPU_ADAPTER_NAME=llvmpipe` selected the software adapter in all probes. An
approved outside-sandbox run selected both host adapters using
`WGPU_ADAPTER_NAME=RADV REMBRANDT` and `WGPU_ADAPTER_NAME=radeonsi`. See
[GPU adapter enablement](gpu-adapter-enablement.md) for inventory, commands,
and per-adapter results.


## Coverage and worldgen status

All parity counts in this table are from WGPU/Naga `30.0.1`, adapter
`llvmpipe (LLVM 22.1.8, 256 bits) (Cpu, Gl)`, with default shader compilation
options. The selector-validation rerun used `WGPU_ADAPTER_NAME=llvmpipe`.

| Stage | Adapter | Result |
|---|---|---|
| Perlin and Smeared Perlin | llvmpipe (CPU/GL) | VERIFIED: 0/60,000 mismatches each |
| Coordinate lattice and gradient indices | llvmpipe (CPU/GL) | VERIFIED: 0/60,000 total |
| Trilinear interpolation and density blend | llvmpipe (CPU/GL) | VERIFIED: 0/60,000 total |
| Synthetic compiled spline + lerp, strict-expression source | llvmpipe (CPU/GL) | VERIFIED: 0/60,000 each |
| Synthetic compiled spline + lerp, explicit FMA | llvmpipe (CPU/GL) | VERIFIED: 0/60,000 each |
| Arbitrary-bit sqrt diagnostic | llvmpipe (CPU/GL) | VERIFIED: 232/60,006; separate issue |
| Perlin and Smeared Perlin | RADV REMBRANDT / Vulkan | VERIFIED: 50,656/60,000 and 48,157/60,000 mismatches |
| Perlin and Smeared Perlin | radeonsi / GL | VERIFIED: 50,296/60,000 and 47,709/60,000 mismatches |
| Coordinate lattice and gradient indices | both AMD adapters | VERIFIED: 0/60,000 total on each |
| Trilinear interpolation / density blend | both AMD adapters | VERIFIED: 27,315/60,000 and 8,190/60,000 mismatches |
| R1–R6 explicit-FMA density-program ladder | RADV REMBRANDT / Vulkan | VERIFIED: max absolute error `2.3841858e-7` across stages |
| R1–R6 explicit-FMA density-program ladder | radeonsi / GL | VERIFIED: 0/60,000 mismatches per stage |
| R1–R6 explicit-FMA density-program ladder | llvmpipe / GL | VERIFIED: 0/60,000 mismatches per stage |
| Arbitrary-bit sqrt diagnostic | both AMD adapters | VERIFIED: 4,767/60,006; separate issue |
| Custom trivial router fixed volume | llvmpipe (CPU/GL) | VERIFIED: (a)/(b)/(c) all bit-exact over 4,096 cells |
| Vanilla NoiseRouter / terrain volume | not run | Phase 48 (WIP branch) measured the overworld `final_density` export: 229 reachable, 136 supported, 93 blockers, 0 of 8 fields export. No GPU terrain path exists; repeat on current `main` (#19146) |

The original unforced AMD Perlin figures above compare a strict CPU arithmetic
reference with ordinary WGSL expressions. In the explicit-FMA contract, RADV
R1–R6 reached maximum absolute error `2.3841858e-7`; the full radeonsi GL and
llvmpipe ladders were bit-exact on their recorded sample sets. Strict RADV
comparisons remain diagnostic and have larger errors (up to `2.4288893e-6`).
ULP distances can be very large near zero, so the hardware bound uses absolute
error. See [RADV divergence diagnosis](radv-divergence.md) for Phase 22.5,
first samples, FTZ and SPIR-V evidence.

There is currently no `gpu-worldgen` feature and no activation command. At the
known seam, `fill_noise_inner` receives an opaque `dyn DensitySampler`; the IR
is not connected to the compiled production router. The fixed-volume result
proves only the custom trivial router subset. The vanilla terrain path remains
CPU-only. Existing probes remain runnable with the commands above; none
enables production GPU world generation.

**[INFERRED] Practical impact:** using one backend consistently for a world
avoids seams caused by mixing these two numeric outputs. If CPU- and GPU-
generated density samples are mixed within one world, values near the block
threshold can theoretically flip isolated cells and create single-voxel seams.
No vanilla volume or voxel-flip rate has been measured, so this is not a
verified terrain prediction.
