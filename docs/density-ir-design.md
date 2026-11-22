# Density program IR design

This document is the design source of truth for a small, additive density
program representation used by CPU/GPU parity probes. The production CPU path
remains unchanged: `get_sampler` optimizes a graph, then `compile_raw` lowers
each definition to an opaque `Arc<dyn DensitySampler>`. A parallel
`export_to_program` reads graph definitions before that loss of structure and
supports only a documented pointwise subset.

## Existing graph and sampler inventory

`GraphNode` is declared in
[`graph/mod.rs`](../crates/horizon-core/src/density/graph/mod.rs:63). Its
definition variants and current disposition are:

| Definition kind | Definition location | Existing compiled behavior | IR disposition |
|---|---|---|---|
| Constant | `graph/mod.rs:65` | `ConstantSampler`, `graph/compiler.rs:298` | Exported as `Constant` |
| Spline | `graph/mod.rs:66` | `SplineSampler`, `graph/compiler.rs:363-380` | `UnsupportedOperation`; endpoint extrapolation/general spline shape are not represented |
| Noise | `graph/mod.rs:67-74` | seeded `NoiseSampler` or `ShiftedNoiseSampler`, compiler `:309-350` | `NoiseKind`; coordinates, scale and seed table are not exported |
| OffsetNoise | `graph/mod.rs:75-78` | `compile_shift_noise`, compiler `:351-357` | `NoiseKind`; no generic seeded-noise program |
| BlendedNoise | `graph/mod.rs:79` | seeded blended sampler, compiler `:358-362` | `NoiseKind`; octave stack and bulk semantics are not represented |
| EndIslands | `graph/mod.rs:80` | seed-bound sampler, compiler `:304-308` | `NoiseKind`; specialized seeded algorithm |
| WorldField | `graph/mod.rs:81` | `ContextBoundSampler`, compiler `:300` | `NeedsContext`; bound `SamplerContext` data |
| BlendDensity | `graph/mod.rs:82` | `BlendDensitySampler`, compiler `:301-303` | `NeedsContext`; world blender is supplied through worker context |
| Reference | `graph/mod.rs:84-87` | recursively compiled, compiler `:299` | Export recurses to the child; holder identity stays upstream |
| Gradient | `graph/mod.rs:88-95` | `GradientSampler`, compiler `:399-406` | `UnsupportedOperation`; tiling and integer-coordinate semantics need tests |
| Unary | `graph/mod.rs:96-99` | `UnarySampler`, compiler `:407-410` | `UnsupportedOperation`; sqrt/log accuracy and NaN rules need policy |
| Binary | `graph/mod.rs:100-104` | `compile_binary`, compiler `:447-449`, `:568+` | Add/Multiply exported; Subtract/Divide/Minimum/Maximum rejected |
| Round | `graph/mod.rs:105-109` | `RoundSampler`, compiler `:411-427` | `UnsupportedOperation`; integer conversion and ties need contract |
| Power | `graph/mod.rs:110-113` | `PowerSampler`, compiler `:428-446` | `UnsupportedOperation`; libm/backend behavior needs policy |
| Clamp | `graph/mod.rs:114-118` | `UnarySampler`, compiler `:450-453` | `UnsupportedOperation`; NaN/min/max behavior needs validation |
| Choice | `graph/mod.rs:119-125` | `RangeChoiceSampler`, compiler `:454-483` | `UnsupportedOperation`; branch selection and bulk order matter |
| Lerp | `graph/mod.rs:126-130` | `LerpSampler`, compiler `:484-513` | Exported when each child exports; endpoint branches preserved |
| Slice | `graph/mod.rs:131-135` | `SliceSampler`, compiler `:514-549` | `NeedsContext`; changes coordinates and axis domain |
| Cache | `graph/mod.rs:136` | preparation `compiler.rs:219-243`, `CachingDensitySampler`; raw rejection `:550` | `NeedsContext`; worker-owned cache IDs and lifetime |
| Interpolated | `graph/mod.rs:137-141` | `InterpolatedSampler`, compiler `:551-553` | `NeedsContext`; requires grid scheduling/context |
| Surface | `graph/mod.rs:142-147` | surface-search sampler, compiler `:554-566` | `NeedsContext`; searches across heights |
| IntervalSelect | `graph/mod.rs:148-152` | `IntervalSelectSampler`, compiler `:384-398` | `UnsupportedOperation`; bounded branch representation not implemented |
| DistanceToPoint | `graph/mod.rs:153-156` | `DistanceToPointSampler`, compiler `:381-383` | `UnsupportedOperation`; metric/overflow contract not implemented |
| Bound sampler | `graph/mod.rs:387-392`; `compiler.rs:294` | returns an already-compiled trait object | `BoundSampler`; original definition is hidden |

The CPU compiler entry is `DensityCompiler::get_sampler` at
`graph/compiler.rs:63-70`: it calls `optimize` (`:215-217`) and `compile_raw`
(`:292+`). `prepare` (`:219-243`) rewrites references and allocates cache
wrappers. `compile_raw` dispatches on every `GraphNode`; it resolves seeded
noise through the compiler-owned `SeededContext` (`:304-362`). The additive
export entry is `export_to_program` (`:75-77`) with its detailed recursive
lowering at `try_export_to_program` (`:81-155`) and complete blocker list in
`export_blockers` (`:157-212`). The router
compiles its eight roots through `DensityCompiler::get_sampler` at
[`router.rs:71-81`](../crates/horizon-world/src/worldgen/router.rs:71).
The final-density root is selected as a trait object by the generator
(`generator.rs:2883-2900`) and reaches
[`fill_noise_inner`](../crates/horizon-world/src/worldgen/terrain.rs:654),
which calls `final_density.sample_volume(...)` at `terrain.rs:675`.

### WGSL representation assessment

WGSL has f32 scalar/vector arithmetic, arrays, branches, integer bit
operations, and explicit `fma`. R1-R5 nodes below can be expressed in that
language once the exact f32 inputs and seeded permutation table are supplied.
This is not equivalent to moving the full production graph: coordinate setup
uses f64 table offsets and Minecraft's wrap/floor behavior; the supported
contract narrows that work on CPU and compares only the resulting f32 leaf
inputs. `PerlinNoise::new` remains the authority for consuming seed draws and
producing offsets/permutation (`noise/mod.rs:37-59`, `noise/perlin.rs:12-20`).
The IR receives that already-seeded table; it never recreates the seeding
algorithm.

Perlin's value arithmetic needs a recorded expression policy. The selected
policy is explicit f32 fused multiply-add: Rust `f32::mul_add` paired with WGSL
`fma()` for gradient dots, fade, interpolation, spline and blend operations.
Endpoint-specialized lerp preserves the production branch behavior. Results
are bit-exact on the tested llvmpipe and radeonsi GL configurations, and within
`2.4e-7` absolute error on the tested RADV Vulkan configuration; all use
WGPU/Naga 30.0.1 and default pipeline options. WGSL does not promise
backend-independent bit-exact results. See [the locked contract and RADV
diagnosis](gpu-noise-parity.md).

## Proposed enum schema and ladder

The additive module is `horizon_core::density::program`; it does not alter
`DensityCompiler` or existing sampler behavior.

```rust
enum DensityProgram {
    Perlin(PerlinProgram), // R1: seeded table plus narrowed-f32 lattice input
    Constant(f32),         // R2 support
    Lerp { alpha: Box<Self>, first: Box<Self>, second: Box<Self> }, // R2
    Spline { input: Box<Self>, points: Vec<SplineKnot> }, // R3 subset
    Offset { input: Box<Self>, delta: [f32; 3] }, // R4 coordinate offset
    Add(Box<Self>, Box<Self>), // R5
    Mul(Box<Self>, Box<Self>), // R5
    Blend { alpha: Box<Self>, first: Box<Self>, second: Box<Self> }, // R5
}
```

R1 takes a `PerlinSample` containing the cell indices, f32 offsets, and
interpolation-Y fraction. This makes the leaf invariant precise and reuses
the existing seeded table, without claiming WGSL f64 coordinate conversion.
R2 evaluates constants and endpoint-specialized scalar lerp. R3 supports a
single-input cubic spline subset with explicit points and derivatives. R4
applies an f32 coordinate delta to the narrowed lattice representation and
must normalize cell crossings identically. R5 adds Add, Mul and density Blend
composition. A future R6 could only be declared when a named trivial world
preset's actual `final_density` definition lowers entirely into those nodes;
it cannot be assumed from synthetic graph coverage.

The CPU lowering is a recursive evaluator over `DensityProgram`, using the
same ordered operations as its WGSL lowering. The GPU lowering is WGSL
code-generation from the enum into a small compute kernel. Code generation is
preferred over a runtime node-kind switch: each probe specializes one known
program, avoids divergent dispatch and buffer-encoded tagged unions, and makes
the emitted expression order inspectable. This is a development/probe format,
not a serialized public API.

## Extension and excluded behavior

Each extension adds one node, an independent expected-value test, CPU/GPU
comparison and a named-adapter result before moving up the ladder. For R6,
first identify a real trivial preset, build its production router, and prove
that the selected `final_density` graph has no excluded nodes. Only then compare
the normal `fill_noise_inner` result with the IR evaluator and GPU output.

The following remain NOT-REPRESENTABLE without a defined lowering contract:

- Cache: `SamplerContext`-owned values, cache namespace/ID, evaluation
  lifetime, recursive detection, dense versus sparse volume behavior.
- World fields / BlendDensity: runtime world-blender data bound through the
  worker context; needs an explicit immutable snapshot input and versioned
  semantics.
- Interpolated and Surface: control the order and coordinates of repeated
  sampling, not merely a pointwise f32 expression.
- Bound nodes: original graph definitions are hidden behind trait objects.
- Seed-dependent special noise (OffsetNoise, BlendedNoise, EndIslands): each
  needs an explicit serialized seed/table representation and proof of vanilla
  draw order; ordinary Perlin's table alone is insufficient.
- Shifted Noise: needs its child samplers, scale order and coordinate conversion
  in the program, beyond R4's simple f32 offset.
- Slice and context-dependent nodes: alter the domain or require chunk/worker
  state unavailable to a pure pointwise program.

These exclusions do not mean the formulas are impossible to write in WGSL;
they mean current compiler/runtime semantics cannot be represented by this
pointwise IR without further explicit inputs or production changes.

## Implementation notes

Phase 14 design and R1-R6 prototype are complete for the covered cases.
`DensityCompiler::export_to_program` is now additive and supports only direct
constant, reference, add, multiply, and lerp definitions whose children are
also supported. `try_export_to_program` reports the first blocking node and a
reason. It does not change `get_sampler` or CPU compilation. Noise, spline,
context, cache, coordinate-domain and other operations remain excluded; see
the export coverage in [the progress log](gpu-noise-parity-progress.md).
The hardware-backed Perlin leaf is materialized in one GPU dispatch before
graph composition, preserving the tested node boundary under shader
optimization. No feature-gated production GPU route exists yet, and no vanilla
overworld router has been exported.

### Export coverage by graph definition

| Graph definition | Export disposition |
|---|---|
| Constant | Supported |
| Reference | Recursively unwraps and exports its child |
| Binary Add / Multiply | Supported |
| Lerp | Supported when every child exports |
| Noise, OffsetNoise, BlendedNoise, EndIslands | `NoiseKind` |
| WorldField, BlendDensity, Cache, Slice, Interpolated, Surface | `NeedsContext` |
| Bound sampler definition | `BoundSampler` |
| Spline, Gradient, Unary, Round, Power, Clamp, Choice | `UnsupportedOperation` |
| Binary Subtract / Divide / Minimum / Maximum | `UnsupportedOperation` |
| IntervalSelect, DistanceToPoint | `UnsupportedOperation` |

The default vanilla overworld router was not evaluated because the prepared
26.3 registry/settings bundle is absent from this checkout. The exporter will
reject a graph when it reaches an unsupported definition, but the exact
vanilla blocker set is **OPEN** until that preset data is available.
