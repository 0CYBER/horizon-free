# Seeded material surfaces

Horizon implements the pinned Java Edition **26.3** material system's terracotta
band generation, band lookup, surface depth and secondary surface-noise query.
These primitives now feed the [chunk surface stage](#chunk-surface-stage).

`ClayBands` generates all 192 entries from the supplied clay-band random stream.
It preserves orange placement, colored-band widths and overwrites, white/gray
adjacency checks and conditional random draws. `MaterialSurface` derives that
stream from the world's root positional factory using `minecraft:clay_bands`.
Its surface-depth query uses a fresh position-derived random stream.

Band lookup rounds the float noise offset with Java's tie and saturation rules,
then uses wrapped integer addition and signed remainder. A negative final index
returns `None`, corresponding to vanilla's array-index failure. Surface-depth
arithmetic promotes the sampled float to double before applying coefficients and
truncating to an integer. Secondary noise similarly retains the float-to-double
conversion.

Run:

```sh
python3 tools/test_material_surface_parity.py
```

The [local report](../benchmarks/rust-material-surface-2026-10-06/scoped-parity.json)
records **122,988 matching output lines per reference JAR**, covering **277 fixtures**
and **5,263 surface-query positions**. The original Java transport invokes actual
`MaterialSystem` methods and its real constructor. Comparisons include both random
algorithms, direct and derived streams, preconsumed random inputs, every band
entry, subsequent random draws, real bundled noise definitions and constant-noise
overrides for rounding boundaries, infinities, NaNs and extreme coordinates.

Reference artifacts and noise resources are authenticated, source/fixture changes
invalidate the report, and raw inputs/outputs stay private under `.local`. CI
includes this comparison on its configured platforms; the local result does not
establish remote outcomes. Subsequent suites cover rule binding, biome-specific
extensions and chunk surface application.

[Vertical anchors](vertical-anchors.md), used by material conditions, now have a
separate differential implementation and codec comparison.

[Numeric material conditions](material-conditions.md) now evaluate surface inputs
with worker-owned caches. [Bundled rule libraries](material-rule-libraries.md)
bind the pinned material definitions and their ore-density samplers.

## Biome-specific column extensions

`MaterialExtensions` implements eroded-badlands pillars and frozen-ocean icebergs.
It preserves noise coordinate arithmetic, temperature-dependent melting, seeded
random draw order, repeated column reads and property-independent block identity.
The column provider controls out-of-height reads and write clipping; failed calls
retain earlier writes. Each extension call is bounded to 1,048,576 iterations.

Run `python3 tools/test_material_extension_parity.py`. The
[local extension report](../benchmarks/rust-material-extension-2026-10-06/scoped-parity.json)
records **233,100 matching lines per JAR** across **410 fixtures** and **3,920
extension calls**. It compares ordered noise/read/write traces and complete
columns, including both random modes, all 67 biome climates, actual seeded noises,
constant threshold/nonfinite inputs, air/water variants and provider failures.
The chunk surface stage integrates these extensions with heightmaps and fluid
postprocessing.


## Chunk surface stage

`build_surface` traverses columns in vanilla X/Z order and scans down each column,
maintaining stone-depth and water-level counters. It applies bound material rules,
eroded-badlands pillars and frozen-ocean icebergs. Writes update both world-generation
heightmaps and append fluid-postprocessing offsets in order, including duplicates.
Generation bounds clip writes while scans retain the chunk's full height. The
worker returns its density context to the caller on success and typed failure;
previous writes remain visible after failure.

Run `python3 tools/test_material_stage_parity.py`. The
[local stage report](../benchmarks/rust-material-stage-2026-10-06/scoped-parity.json)
records **6,733,382 matching output lines per reference JAR**, covering **79 fixtures**,
**42 terrain fills**, **42 procedural carving passes**, **106 surface builds** and **21,757,952 block values**. The reference invokes actual
`MaterialSystem.buildSurface` on vanilla `ProtoChunk` instances. Comparisons include
complete sections, packed heightmaps, ordered postprocessing and biome queries.
Fixtures exercise both random modes, the bundled rule registries, real seeded
noises, empty and populated chunks, cavities, nondefault solids, clipped writes,
primed and lazy heightmaps, and repeated builds sharing density contexts.
Generated fixtures run actual `NoiseBasedChunkGenerator.doFill` followed by surface
application on the same native noise chunk. Rust shares a compiler and worker
context across both stages. All seven bundled terrain routers and aquifer
configurations are tested with multiple seeds and distant positive/negative
coordinates, comparing full chunk state before and after surface application.

Synthetic initial chunks use a supplied block-position biome resolver. Generated
terrain fixtures first generate a 3×3 biome neighborhood through actual
`ChunkGenerator.doCreateBiomes`, using multi-noise presets or the End source.
The comparison checks **378 biome chunks** and **400,896 biome cells**, then
performs seeded `BiomeManager` lookups against those stored neighbor biomes.
Both block and quart lookup traces are compared. Rust imports independently
authenticated preset tables; their builders remain separate work.
Generated fixtures additionally select and run biome carvers across the 17×17
origin neighborhood, comparing uncached biome queries and the complete terrain
state after procedural carving. They then apply explicit masks twice as a stress
check. See [carving integration](carving.md#biome-selected-neighboring-carvers).
The neighborhood decodes its source JSON through the registry-bound biome-source
catalog and uses the shared resolved-source worker API. Block air/fluid metadata
is imported from both authenticated JARs. Native preset builders, full inline biome
codecs, feature/structure stages, nonempty blending, lighting
and complete generated worlds
remain pending. A separate Rust test verifies that successful builds, eager binding
failures and late biome-provider failures preserve density caches and world fields,
and that late failure retains prior chunk writes and fluid offsets.
CI includes the stage gate; remote results have not been observed.


## Surface material beneath carved blocks

`top_material` binds a fresh one-cell material context at the supplied position,
using clamped neighboring heightmap gradients and one stone block above and below.
Submerged queries set water height to Y+1; dry queries use the vanilla sentinel.
Possible-biome restrictions are omitted, as in vanilla's method. The returned
state is left for the carving stage to apply. Both successful and failed queries
retain the caller's density context; the query itself does not change blocks.

The surface-stage comparison includes **7,584 top-material queries** against
actual `MaterialSystem.topMaterial`, at column edges and interiors, around sea
level and immediately outside vertical bounds, in both dry and submerged contexts.
The latest combined report records **6,733,382 matching lines per JAR**. Unit tests
also check failure-state retention and the omission of possible-biome shortcuts.
