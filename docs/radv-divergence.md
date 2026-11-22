# RADV Perlin divergence

## Verdict: TOLERANCE on RADV; exact on the tested radeonsi GL path

**[VERIFIED]** On AMD Radeon Graphics (RADV REMBRANDT, IntegratedGpu, Vulkan),
WGPU/Naga 30.0.1, `shader_f64=true`, default pipeline options, strict Rust
`a*b+c` vs ordinary WGSL differs at R1 for 50,626/60,000 samples (max absolute
error `2.4288893e-6`; max 339,581 ULP near zero). Under the corrected explicit
FMA shader (CPU `f32::mul_add`, WGSL `fma()` for gradient dots, fade, lerp,
spline and blend), R1 differs for 8,986/60,000 samples, with max absolute
error `2.3841858e-7`. It is tolerance-bounded, not bit-exact.

**[VERIFIED]** Under that explicit-FMA contract, RADV R1–R6 maximum absolute
errors were: R1 `2.3841858e-7`; R2 interior `1.4901161e-7`, alpha zero
`2.3841858e-7`, alpha one `0`; R3 `1.7881393e-7`; R4 `2.0861626e-7`; R5
`1.0430813e-7`; R6 `0`. Thus the measured maximum is below the locked
`2.4e-7` bound for this 60,000-sample ladder. It is an empirical bound, not a
portable WGPU guarantee.

**[VERIFIED]** The explicit-FMA R1–R6 probes on llvmpipe (LLVM 22.1.8,
256 bits, Cpu, GL), WGPU/Naga 30.0.1, `shader_f64=false`, default options,
remain bit-exact at all rungs (0/60,000 differences). On AMD Radeon Graphics
(radeonsi, Rembrandt, ACO, Other, GL), the same WGPU/Naga versions, options,
and explicit-FMA contract also had 0/60,000 mismatches at every rung. The
synthetic fade test shows both tested AMD and software adapters flush a subnormal `x^3`
intermediate: for `x=2^-49`, strict CPU fade returns `0x00000028`, while the
GPU returns zero and matches a CPU reference that flushes that intermediate.
Thus FTZ is a real shared behavior, but it does not explain the large RADV-only
R1 discrepancy.

## Localization

**[VERIFIED]** RADV’s lattice subprobe had zero mismatches in coordinate wrap,
shift, floor/cell, fraction, permutation and corner-gradient index stages
(60,000 samples). The divergence begins in floating-point Perlin evaluation,
not the integer hash path.

**[VERIFIED]** Naga 30.0.1's emitted SPIR-V for the strict source contains
separate `OpFMul`/`OpFAdd` operations in the Perlin gradient accumulation. The
explicit-FMA variant contains `OpExtInst Fma` for requested fused operations.
This rules out Naga contracting the strict WGSL source during its SPIR-V
emission. It does not reveal later RADV compiler transformations. WGSL leaves
ordinary expression fusion to backend evaluation, so the observed RADV/GL
behavior may depend on backend arithmetic choices; a driver miscompile is not
established.

## Phase 22.5 — backend experiments

**[VERIFIED]** Running `RADV_DEBUG=noopt` on RADV REMBRANDT/Vulkan with
WGPU/Naga 30.0.1 and default pipeline options left strict R1 at 50,626/60,000
and explicit-FMA R1 at 8,986/60,000 (max absolute `2.3841858e-7`). Disabling
RADV optimization did not improve the result or restore bit equality.

**[VERIFIED]** On AMD Radeon Graphics (radeonsi, Rembrandt, ACO, Other, GL),
WGPU/Naga 30.0.1, `shader_f64=false`, default options, strict R1 remained
50,296/60,000 mismatches (max absolute `2.4288893e-6`), while explicit-FMA R1
was bit-exact at 0/60,000. This reopens bit-exactness for that specific GL
adapter and arithmetic path; it does not imply cross-adapter hardware exactness.

## Contract and scope

For these probes, report both output-bit mismatches and maximum absolute error.
Use bit equality for llvmpipe and the tested radeonsi GL claims. On RADV
REMBRANDT/Vulkan use the `2.4e-7` absolute bound only for the explicit-FMA R1–R6
code and deterministic sample set; do not generalize it to other adapters,
driver versions, or the production NoiseRouter. ULP alone is misleading near
zero. The separate
232-case sqrt drift from the probe-vs-narrowed-f64 oracle is unrelated and is
not part of this Perlin verdict.

**[OPEN]** There is no proof here of backend-independent error bounds, actual
RADV machine-code contraction behavior, or vanilla worldgen parity. Production
worldgen has only a limited CPU graph exporter; it has no WGPU execution path.
