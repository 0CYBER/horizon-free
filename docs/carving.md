# Carving

Horizon implements Java Edition **26.3** carving-mask storage and traversal.
`CarvingMask` packs cells in Y/Z/X order into 64-bit words. Its column iterator
finds contiguous marked runs and splits them into inclusive vertical intervals,
using word scans without allocating a list of individual cells or columns.

`try_visit_mut` retains vanilla callback behavior: adding cells during traversal
changes subsequent run searches, while an already selected run keeps its original
boundaries. Callback failures stop immediately and retain prior mutations.
Coordinate arithmetic wraps like Java, including positive out-of-range aliases;
negative indices return an error. Rust bounds height to 1–4,064 and positive growth
to 2^24 bits. Invalid construction and allocation limits are explicit errors.

Run `python3 tools/test_carving_mask_parity.py`. The
[local report](../benchmarks/rust-carving-mask-2026-10-06/scoped-parity.json)
records **45,742 matching output lines per reference JAR**, across **28 fixtures**
and **6,988 operations**. The native `CarvingMask` supplies all reference results.
The corpus includes sparse and fully filled masks, duplicate insertions, word and
column boundaries, extreme signed coordinates, positive aliases, negative-index
failures, and successful or failing callbacks that add cells during traversal.
All corpus indices remain below 2^20. Reference artifacts remain private and
source/fixture hashes guard validation.

This comparison covers masks. The combined generation check below also covers
neighbor biome selection and carver seeding; complete generated worlds remain pending.

## Applying masks to terrain

`apply_carving_mask` traverses each marked interval from top to bottom, applying
an optional caller-supplied predicate before block or aquifer queries. The
resolved `minecraft:uncarvable` tag prevents replacement. Grass and mycelium mark
a surface encountered within the current interval, including when the aquifer
returns no replacement. Subsequent exposed dirt beneath carved cells is restored
through `top_material`. Block changes update WG heightmaps; fluid postprocessing
retains vanilla order and duplicates. Later errors preserve prior effects.

The [combined surface-stage report](../benchmarks/rust-material-stage-2026-10-06/scoped-parity.json)
records **6,733,382 matching lines per JAR**, including **158 carving passes** and
**21,757,952 block values** across noise filling, surface building and carving.
Actual `NoiseBasedChunkGenerator.applyCarvingMask` supplies the reference results;
its existing aquifer is wrapped to trace ordered requests, results and fluid
scheduling decisions. The test applies explicit masks, filtered and unfiltered,
with grass/mycelium/dirt and uncarvable blocks, repeated writes and disjoint runs.
The filter is synthetic; native blending-filter generation remains pending.

Run `python3 tools/test_material_stage_parity.py`. Unit tests additionally verify
that filters and uncarvable tags skip aquifer calls, a surface observation survives
an absent aquifer result, and later aquifer failure preserves earlier surface
restoration and its fluid postprocessing even when aquifer scheduling is disabled.



## Shared shape primitives

`carve_ellipsoid` visits candidate cells in X/Z/descending-Y order, retaining
vanilla's exclusive lower Y bound, Y−0.5 normalization, chunk clipping and ordered
skip callbacks. Output failures preserve earlier marked cells. `can_reach` uses
wrapped step differences and float-rounded thickness before double-distance math.
Traversal has a 2^24-candidate work bound.

Run `python3 tools/test_carving_geometry_parity.py`. The
[geometry report](../benchmarks/rust-carving-geometry-2026-10-06/scoped-parity.json)
records **106,699 matching lines per JAR**, across **690 ellipsoids** and **5,536
reachability queries**. Actual `WorldCarver.carveEllipsoid` and `canReach` supply
the reference results. Tests compare normalized skip inputs, marked-cell order,
partial failures and final mask intervals, with distant/overflowing coordinates,
clipped bounds, degenerate radii, NaNs, infinities and subnormals. NaN trace values
are canonicalized. The seeded cave/canyon paths below use these primitives. Gaussian provider codecs remain separate work.


## Seeded cave and canyon paths

`CaveCarver` decodes cave definitions and reproduces start selection, rooms,
thickness bias, randomized tunnel movement and branching. `CanyonCarver` decodes
canyon definitions and reproduces height-dependent width factors, varying radii
and randomized path movement. Both preserve float/double operation order and
vanilla's quantized trigonometry. Their local path RNG is always the legacy
algorithm, independently of the caller's outer random mode.

Run `python3 tools/test_cave_carver_parity.py` and
`python3 tools/test_canyon_carver_parity.py`. The
[cave report](../benchmarks/rust-cave-carver-2026-10-06/scoped-parity.json) records
**882,005 matching lines per JAR** across **415 fixtures** and **739 requested
carve invocations**. All three bundled cave definitions are independently
imported from both JARs. The
[canyon report](../benchmarks/rust-canyon-carver-2026-10-06/scoped-parity.json)
records **1,094,517 matching lines per JAR** for the bundled canyon and custom
shapes across four generation heights.

The native `WorldCarver.DIRECT_CODEC` and actual cave/canyon methods supply
reference results. Comparisons include ordered cell writes with duplicates,
mask intervals, partial output failures and subsequent random longs. Both random
modes, repeated invocation, distant/overflowing coordinates and neighboring/far
origins are covered. Custom shapes force branching, floor and width variation,
degenerate radii and failure boundaries; malformed selected fields are tested.

Rust bounds a cave call to 4,096 starts, a canyon to 4,096 steps and 4,064 width
entries, and either call to 2^24 candidate cells through a shared traversal budget.
Budget exhaustion retains prior mask writes. Gaussian provider codecs,
encoding, blending and complete worlds remain pending; these suites invoke
individual carvers directly. The combined stage check covers neighbor selection and seeding.


## Biome-selected neighboring carvers

`CarverCatalog` resolves named and inline carvers while preserving ordered holder
lists and repeated references. `generate_carving_mask` visits the 17×17 origin
neighborhood in X/Z order. Each carver receives a legacy random stream initialized
from the world seed plus its list ordinal and the origin chunk coordinates, then
runs its native start-selection and shape path. The mask clips below the ceiling
with vanilla's seven-block allowance, or without that allowance for upgrading
chunks. Source errors stop generation before mask application.

The combined stage report includes **42 procedural carving passes** through actual
`NoiseBasedChunkGenerator.generateCarvers`, after noise filling and surface work
and before the explicit-mask stress passes. Biome selection uses uncached quart
queries and retains the source's prior search state from biome filling. Carved
surface restoration switches fuzzy lookup to that raw source. All four bundled
carvers and all seven terrain settings are available to biome selection; the
native biome settings retain actual carver lists with empty placed-feature lists.

Unit tests cover origin order with coordinate overflow, upgrading bounds,
invalid bounds before source calls, early source failure and duplicate holder
identity. Tagged carver sets, region-neighbor caching, nonempty blending,
retrogen integration, placed features and complete generated worlds remain pending.
