# Structure terrain-adaptation density

`horizon_core::density::Beardifier` implements the pinned Java Edition **26.3**
structure-density sampler from supplied ordered rigid pieces, jigsaw junctions
and an affected bounding box. It supports `NONE`, `BURY`, `BEARD_THIN`, `BEARD_BOX`
and `ENCAPSULATE` adjustments. Live structure discovery, filtering and placement
remain separate work.

The sampler retains float accumulation order, wrapped coordinate arithmetic,
ground-level deltas, the 24-by-24-by-24 kernel and vanilla's inverse-square-root
approximation. Its kernel is computed once from the formula. No extracted vanilla
kernel data is stored in the source tree. The immutable sampler can be attached
to `WorldField::Beardifier` and shared between worker contexts.

Scalar reads return zero outside the affected box. Bulk reads clear the entire
output, including an oversized tail, then scan the clipped strided volume.
Vanilla floors the lower clipping index rather than rounding it upward, so a
first bulk sample can be outside the scalar bounds. Horizon preserves this
scalar/bulk distinction. A missing affected box produces zero even if component
lists are nonempty. Bounds supplied to the Rust constructor must already be
normalized; short output buffers return a typed error before mutation.

Run:

```sh
python3 tools/test_structure_density_parity.py
```

The [local report](../benchmarks/rust-structure-density-2026-10-06/scoped-parity.json)
records **1,360,095 matching output lines per reference JAR** across **183 fixtures**,
**28,093 scalar queries** and **339 volumes**. The original Java adapter calls the
actual `Beardifier` constructor and samplers. Cases include every adjustment mode,
the full junction kernel neighborhood, ground offsets, overlapping components,
reversed accumulation order, null/empty bounds, floor clipping, output tails and
extreme wrapped coordinates. Float outputs are compared by raw bits with
canonical NaNs.

The report records the authenticated JARs, source and fixture hashes. Raw outputs
stay private under `.local`. CI runs the comparison on its configured platforms;
the local `Math.pow`-derived kernel check does not establish cross-platform math
parity. The [noise-fill comparison](terrain-filling.md) also checks supplied structure
density through final-density and aquifer graphs. Structure-start selection,
invalid-box repair and complete structure terrain generation remain pending.
