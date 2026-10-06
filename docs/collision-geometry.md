# Horizon collision geometry

The version target is Minecraft Java Edition **26.3**. Geometry is a simulation
foundation; Horizon has a native client for inspecting a controlled block scene.
Live gameplay and entity simulation remain unfinished.

`horizon-core::geometry` provides `Vec3`, `Aabb`, `Axis`, `Direction`, `BlockHit`,
and immutable `VoxelShape` values. Coordinates are doubles. Arithmetic keeps
vanilla's operation order, signed zero, NaN comparisons, normalization threshold,
and collision epsilon. It does not enable approximate or fused simulation math.

Implemented vector operations include addition/subtraction, vector-to, component
products, scaling, normalization, dot/cross products, interpolation, projection,
distance/length, horizontal vectors and finite/equality checks. Rotations, look
directions, alignment, axis replacement, direction offsets, and pitch/yaw extraction
are also implemented. The full Vec3 API and entity integration remain absent.

Trig uses vanilla's 65,536-entry float sine table with double angle scaling and
Java long narrowing. The table is generated from the pinned formula in Rust;
the geometry suite compares every generated entry against the runtime table.
Inverse trig narrows to float at vanilla's conversion points. These comparisons
validate the tested platform; other platforms need the same bit comparisons.

Bounding boxes normalize endpoints with Java-compatible min/max. Queries include
half-open containment, strict intersection, transforms, distances, ray clipping,
block-coordinate construction with wrapping integer endpoints, setters, next-value
deflation, and swept-box collision tests. A disjoint `intersect` preserves vanilla's
constructor normalization rather than introducing a different empty-box rule.

Shapes keep exact coordinate arrays and a packed occupancy grid. Greedy boxes are
cached once in vanilla order. Collision-distance queries use occupied grid cells;
ray and closest-point queries reuse the cached boxes. Column minima/maxima use
vanilla's cyclic perpendicular coordinates and infinity for empty/outside cells.
Cube shapes retain arithmetic
lookup; moved shapes use array lookup even for a zero offset. Ray hits preserve
face tie ordering, endpoint exclusion, tolerances, and the 0.001 inside-hit nudge.
Boolean shape joins and overlap predicates preserve cube, identical,
disjoint and epsilon-merger behavior. Shape operations reject truth tables that
fill two empty cells, as vanilla does. Per-axis cube-coordinate metadata survives
partial merges and is checked when importing cached shapes. Box construction
preserves thin-box rejection and up to 1/8 cube quantization. Optimized joins
reconstruct greedy boxes in vanilla order. Face extraction is implemented; edge extraction and shape rotations remain absent.

Discrete axis extrema are retained separately from occupied cells. A box can
quantize to no occupied cells while retaining a finite minimum/maximum on other
axes. That state affects vanilla's lower-block floor-height lookup. Empty joins
also preserve their integer sentinel extrema; optimization and moving an empty
shape return the canonical empty shape.

Shape clones preserve identity and share immutable coordinate, occupancy and box
storage. New shapes keep separate identities even when their grids are equal.
This preserves alias-sensitive join shortcuts and avoids copying large grids
when an operation returns an existing shape. The cache importer shares equal
grid storage while retaining the distinct vanilla shape identities.

`resolve_shape_movement` ports vanilla's movement resolver over a supplied shape
list: Y first, then horizontal axes in movement-magnitude order, translating the
box by resolved motion before each axis query. `candidate_step_heights` preserves
float narrowing, duplicate identity, current-Y exclusion, and sorted candidate
heights. `resolve_step_movement` adds the initial collision and step-up decision,
including landing-height adjustment, a second collider query, and the first
candidate improving horizontal movement. `entity_collision_bounds` preserves
the initial entity query bounds. These helpers take caller-selected colliders;
they do not choose world/entity shapes, set grounding, change velocity, or apply
inside-block effects.

The step-up corpus calls the actual private `Entity.collide` method for **17,680
cases** per JAR. Reference fixtures supply entity inputs and controlled
block/entity/border shape lists; vanilla performs the movement decisions. Query
bounds and results are compared bit for bit. This verifies the decision with
those inputs, not live world selection or entity state transitions.

`block_floor_height` returns the selected shape's maximum Y, or lazily queries
the lower shape only when the current one is empty. Its 12,000-case corpus also
checks the lower supplier's invocation count against `BlockGetter`.

`traverse_blocks` visits blocks in vanilla ray order, including its endpoint nudge,
strict corner comparisons, early callback return, integer wrap and signed-zero
endpoint equality. It accepts finite rays and an explicit visit budget. Invalid
geometry or exceeding that budget returns a typed error; earlier callbacks remain
applied. Actual block/fluid ray selection is a separate integration task.

Inclusive block-box iterators preserve X/Y/Z order, while directional iterators
preserve vanilla's movement-dependent axis order and corner signs. AABB iteration
includes floored maximum faces. They return independent position values and
reject true volumes above the positive Java-int domain, instead of reproducing
overflowed Java iteration counts.

`sweep_intersected_blocks` ports movement-path visits from
`BlockGetter.forEachBlockIntersectedBetween`: tiny movement, the initial box,
ray-clipped footprints, packed-position deduplication, final box, step indices,
and early callback stops. Stationary/tiny movement avoids a deduplication-set
allocation. Inputs and computed geometry must be finite; a caller-selected work
budget counts ray steps and iterator candidates, including suppressed duplicates.
Earlier callbacks are not rolled back on an error. Block effects still need
their own implementations and integration with entity movement.

`horizon-world::collision::CachedCollisionRegistry` imports a private local export
of vanilla cache data. The export includes DataVersion, all state descriptors,
deduplicated shapes, coordinate-kind metadata, discrete extrema, and a per-state
cache index. Import verifies every state ID
against the block registry. Section lookup also requires the same borrowed registry.
Imported nonempty extrema must agree with occupancy; empty extrema must be in
the grid's domain or match vanilla's empty-join sentinel triple. Extrema form part
of the grid-sharing key, so equal occupancy does not erase different metadata.
There is no fallback shape for an unsupported state: `None` means block behavior
must select it, while a cached empty shape is a valid `Some` value.

This covers the cached branch of the **no-context** `BlockState.getCollisionShape`
method. Vanilla's overload taking `CollisionContext` always calls block behavior;
the cache cannot establish entity collision parity. Dynamic and ordinary entity-context selection are implemented as described below.
Experimental minecart contexts, world borders, movement integration, fluid
simulation, block effects and live collision broad-phase queries remain pending.

Run the comparisons after preparing the pinned artifacts:

```sh
python3 tools/test_geometry_parity.py
python3 tools/test_collision_parity.py
```

Both suites invoke actual vanilla methods from the authenticated client and server
JARs. They compare canonical double bits, including infinities/NaNs, rather than
using a numerical tolerance. The geometry corpus includes empty and sparse grids,
greedy decomposition, overlapping boxes, rays, tiny movements and IEEE boundaries.
It compares **1,320,187 output lines per JAR**, including **2,400 integer-box
iterator cases**, **2,400 AABB iterator cases**, and **9,022 movement-path traces**.
The registry suite exercises cached shape queries through resident section IDs,
translation at negative and world-border coordinates, and invalid import rejection.
Its export records **35,524 cached states**, **199 uncached states**, **6,093
shape identities**, and **330 distinct grids**. The comparison runs **213,144 queries** plus state/box/identity checks,
producing **1,097,456 matching output lines per JAR**. These counts describe this
branch and corpus; they are not a gameplay coverage percentage.

Raw grids admit at most 1,048,576 cells with strictly increasing, non-NaN
coordinates. Oversized or malformed imports/joins return typed errors. This
bounded library API does not reproduce Java's complete exception/object surface.

All exported data, test inputs, reference output, and compiled adapters remain
under gitignored `.local/`. Success reports record exact sources, inputs, outputs,
and reference hashes. A runner removes its prior success report before running
and rejects edits to the captured sources during validation. Passing these suites
does not prove whole-game parity or any FPS/render-distance improvement.

Checked-in [geometry metadata](../benchmarks/rust-geometry-2026-10-05/scoped-parity.json)
and [cached-collision metadata](../benchmarks/rust-collision-2026-10-05/scoped-parity.json)
identify their tested source versions. Reports cover their recorded snapshots;
later changes require fresh validation.

Entity obstruction now evaluates ordered candidate snapshots, excluded entity
identity, spectator/removal/building flags, shared root-vehicle identity and
voxel-shape intersection. Empty shapes skip the query; candidate collection
finishes before obstruction evaluation. Unit-block queries translate in floating
point, including integer-coordinate boundaries. Run
`python3 tools/test_entity_obstruction_parity.py` to compare against the actual
vanilla `EntityGetter` default methods and entity metadata queries.

The [entity-obstruction report](../benchmarks/rust-entity-obstruction-2026-10-06/scoped-parity.json)
records **15,121 matching output lines per JAR** over composite shapes, boundary
contacts, thin boxes, metadata filters, vehicle roots and injected query failures.
The Java fixture privately allocates entity snapshots without running their
constructors. Entity lifecycle, spatial indexing and vehicle-tree maintenance
remain separate implementation work. Workspace tests, warning-free Clippy and
**138 Python tests** passed at this snapshot.

Legacy block solidity now uses explicit definition overrides and cached collision
bounds, with force-on precedence and native mean-size/height thresholds. The
no-context replaceable and liquid flags are kept independently. Solid rendering
uses the occlusion definition flag and a full-shape test over imported raw
occlusion geometry. Import validates the pinned
DataVersion, full state identities and complete block definitions before building
a shared property table. Predicates query this table in constant time.

`python3 tools/test_block_properties_parity.py` compares all **35,723 states** and
**18,096 synthetic cache/flag/shape cases**, including adjacent floating-point values
around solidity thresholds and both occlusion flag values. The [block-properties report](../benchmarks/rust-block-properties-2026-10-06/scoped-parity.json)
records **197,180 matching lines per JAR**. Definition flags, collision and raw
occlusion data are private runtime inputs; Rust calculates solidity and solid
rendering and caches full-face occlusion masks. Every state and synthetic shape
is compared on all six faces; source and fixture hashes are recorded.
Item/fluid-context replacement and collision with live entities remain separate work.

Face selection now preserves native cube-axis shortcuts, epsilon sample points,
slice coordinates and clamped parent extrema, including sparse slices whose
native bounds differ from occupied boxes. Repeated queries share face identities.
The FULL, CENTER and RIGID support tests execute against the selected shape.
`python3 tools/test_face_support_parity.py` compares **1,615 input shapes** and
nested faces, producing **595,686 matching lines per JAR** in the
[face-support report](../benchmarks/rust-face-support-2026-10-06/scoped-parity.json).
The [refreshed geometry report](../benchmarks/rust-geometry-2026-10-06/scoped-parity.json)
also retains **1,320,187 matching lines per JAR** after face slicing was added.
Block-specific support-shape selection is a separate layer.

Registered support caches now derive FULL/CENTER/RIGID masks from privately
imported support geometry and share results by shape identity. The importer
checks the data kind and full state identities. `None` explicitly requests
uncached behavior; it never becomes an assumed unsupported face.
`python3 tools/test_block_support_parity.py` compares **35,524 cached states**
across six directions and three support kinds, while retaining **199 uncached
states**. The [cached-support report](../benchmarks/rust-block-support-2026-10-06/scoped-parity.json)
records **248,867 matching lines per JAR**.

Face-cache storage is now lazy and attached to shape identity. Clones share it;
new identities and moved shapes start with an empty cache. A regression test
checks that cached faces do not retain their parent after it is dropped. The
face and geometry differential suites passed again after this allocation change,
as did workspace tests, Clippy and **140 Python tests**. No FPS claim is inferred
from these library checks.

Dynamic support selection now handles bamboo collision offsets, dripstone and
sulfur-spike thickness/direction, the empty-context scaffolding support shape,
powder snow, and shulker animation-state support faces. Moving-piston lookup
preserves missing/wrong block-entity behavior and delegates active entity shape
calculation to the world. The [dynamic-support report](../benchmarks/rust-dynamic-support-2026-10-06/scoped-parity.json)
records **4,867,780 matching lines per JAR** over **107,308 fixtures** covering all
**199 uncached states**, coordinate boundaries, block-entity snapshots and query
failures. Run `python3 tools/test_dynamic_support_parity.py` to reproduce it.
The current suite also compares no-entity-context collision geometry in **60,161
fixtures**, including **13,014 shulker progress fixtures**. Shulker collision
bounds retain native float interpolation and differ from animation-state support
faces. `BlockCollisionRegistry` dispatches cached and dynamic collision queries;
entity movement contexts and block-entity lifecycle remain separate work.
Workspace tests, warning-free Clippy and seven focused Python tests passed.

Moving-piston collision now selects stationary extended bases and moving heads,
translates shapes using native float progress arithmetic, and honors the
movement-direction collision exclusion. Interpolation preserves float operation
order, including negative partial ticks and IEEE edge cases. The separate
collision-related state used for entity pushing retains its own sticky/head
rules; entity pushing itself is not implemented by this module.

`python3 tools/test_piston_shape_parity.py` compares **102,315 fixtures**,
including every registered moved state and **10,000 raw float interpolation
cases**. The [piston-shape report](../benchmarks/rust-piston-shape-2026-10-06/scoped-parity.json)
records **1,096,811 matching lines per JAR**. This corpus uses absent block
entities for moved-block collision selection. Workspace tests, warning-free
Clippy and **142 Python tests** passed. Tick progression and live world mutation
remain separate work.

The refreshed dynamic-support corpus also includes **9,216 active piston
snapshots**, composing block-entity lookup with piston collision generation and
all support tests. It compares extension/retraction, source heads, progress
boundaries, collision exclusions and query failures. These snapshots do not
exercise piston ticks, pushed entities or chunk updates.

The archived block-survival snapshot executes 51 rule kinds with 63 distinct rule/tag
bindings over registry-bound solid flags, air identity, support tags and
FULL/CENTER/RIGID geometry and empty-context collision. The verified snapshot covers
**all 1,286 pinned block definitions and 35,723 states**. Candidate
properties select attachment direction, hanging state, door/plant halves and
pale moss carpet's serialized `bottom` property. Pressure plates and cactus
flowers preserve repeated neighbor reads. Downward center support applies the
unstable-bottom-center tag only where native helpers do. Upper doors require the
same block below; upper double plants also require its lower half. Crop survival
checks brightness against 8 before querying support; farmland preserves its
conditional second read above, and dripleaf stems read both neighbors. Sugar cane,
lily pads, frogspawn, spore blossoms, small dripleaves and tall seagrass preserve
their independent fluid queries, source/full-water requirements and tag checks.
Growing plants preserve head/body identity and kelp support restrictions.
Scaffolding recomputes neighbor distance in native order; chorus plants and flowers
preserve branching reads and early exits. Piston heads validate base type, facing
and extension. Mangrove propagules, speleothems, pitcher crops, bells and shelf
mushrooms use their version-specific attachment rules. Multiface blocks, vines,
hanging moss, snow layers and sea pickles preserve support/collision query order.
Fire uses imported ignition odds and waterlogging; cactus checks horizontal solids
and lava before its support and upper liquid state. Paths recognize fence gates,
and mushrooms preserve the light-override tag and solid-render requirement.

Interrupted runs can be resumed with `python3 tools/test_block_survival_parity.py --resume`.
The runner verifies source, environment, fixture and compiled-artifact hashes
before reusing completed stages. A changed output is regenerated; changed inputs
require a fresh run without `--resume`. Checkpoints stay under `.local`, and
a parity report is written only after both native comparisons pass.

`python3 tools/test_block_survival_parity.py` compares actual native `canSurvive`
calls in both pinned JARs. The [block-survival report](../benchmarks/rust-block-survival-2026-10-06/scoped-parity.json)
records **35,998,488 matching output lines per JAR** across **9,229,463 fixtures**.
Each supported candidate state is exercised; behavior representatives are tested
against every neighbor state, changing repeated reads, integer-coordinate
wrapping, absent/closed/open shulker snapshots and injected block/entity query
failures, plus light thresholds, independent fluid sequences and fluid-query
failures. Native method ownership
and support-tag fields supply private
definition metadata; survival results are calculated in Rust. Workspace tests,
Clippy and the **147-test Python suite** passed. This establishes the recorded
survival scope; full placement, neighbor updates, scheduled behavior and complete
block gameplay remain pending.


Collision contexts now preserve empty-context fallback heights, position-only
queries, and captured entity position, descending, main-hand and placement flags.
Height comparisons retain the native float epsilon widened to double. Null
placement factories use `-Double.MAX_VALUE`; `withPosition` sets the placement
flag and ignores the supplied height when the entity is absent. Empty item stacks
match AIR in entity/empty contexts, while position-only contexts never report a
held item. Living-entity fluid callbacks run before the same-family exclusion,
with the second fluid as their argument, and propagate failures.

`python3 tools/test_collision_context_parity.py` exercises actual native factories
and queries over **8,338 context snapshots**, comparing **225,126 output lines per
JAR**. The [context report](../benchmarks/rust-collision-context-2026-10-06/scoped-parity.json)
records the pinned artifacts and exact source/input/output hashes. The corpus
includes adjacent representable values around height thresholds, empty shapes,
integer-coordinate extremes, NaN/infinity, null entities, item count boundaries,
all five fluid types and callback failures. Native entity and stack snapshots are
allocated privately without their constructors; this checks query behavior, not
entity construction or inventory lifecycle. Inventory changes after context
capture, experimental minecart contexts and live movement integration remain
pending. Contextual block-shape selection is being added separately.


`ScaffoldingCollision` now selects stable scaffold geometry, the unstable bottom
platform, or an empty shape using the collision context. Placement contexts are
always empty. Descending affects the top surface check; the bottom platform also
requires nonzero support distance, the bottom flag and the lower height check.
No-context support and contextual collision share the same immutable stable shape.

`python3 tools/test_scaffolding_collision_parity.py` compares all **32 states**
across **56,448 fixtures**, producing **524,300 matching lines per JAR** in the
[scaffolding report](../benchmarks/rust-scaffolding-collision-2026-10-06/scoped-parity.json).
The native context dispatch executes against a world proxy that rejects all
queries. Full shape coordinates, bounds, ordered boxes and cube metadata match
for empty, position, entity and placement contexts, including adjacent doubles
at both height thresholds. Block placement, distance updates, collapse and live
entity motion through scaffolding are separate work.


`LiquidCollision` implements contextual water/lava collision selection. The
always-fluid flag returns a full cube before querying entities or neighbors.
Otherwise, flowing block levels and nonliving contexts return empty. Source
blocks request the living entity's standing shape, check its height, then read the
fluid above and invoke the entity's standing predicate. Empty candidate shapes
still follow those query rules. Above-block coordinates wrap as Java integers.

`python3 tools/test_liquid_collision_parity.py` compares all **32 water/lava block
states**, all **33 possible above-block liquid/empty states**, full selected shape
data and ordered callback traces, including failures at each query. The
[liquid-collision report](../benchmarks/rust-liquid-collision-2026-10-06/scoped-parity.json)
records **571,179 matching lines per JAR**. Standing shapes and fluid abilities are
controlled inputs in this suite; each entity type still needs its implementation.
Fluid spreading, swimming, drowning and live movement remain pending.


Powder snow now selects the native `0.9f`-high falling shape when fall distance
exceeds 2.5, a full cube for falling-block entities, and a full cube for entities
that can walk on it, are above it and are not descending. Placement and absent
entity contexts are empty. Walkability checks the entity-type tag first, then
leather boots only for living entities; successful tags skip equipment queries.
The API takes current entity metadata and lazy tag/equipment providers.

`python3 tools/test_powder_snow_collision_parity.py` compares **63,336 fixtures**
and **599,180 matching lines per JAR** in the
[powder-snow report](../benchmarks/rust-powder-snow-collision-2026-10-06/scoped-parity.json).
The native block method and walkability predicate execute over controlled entity
snapshots. The corpus covers fall-distance and top-height boundaries, nonfinite
values, falling-block instances, item counts, query failures and skipped queries.
Entity tag loading, freezing, sinking and live movement remain separate work.


`BlockCollisionRegistry::collision_shape_with_context` now combines the contextual
scaffolding, liquid and powder-snow rules with the existing cached/dynamic paths.
It dispatches these overrides before consulting cached geometry. The pinned
native audit checks **35,492 context-independent cached states** at seven positions
with both interface and entity contexts that reject every query: each returns the
identical cache object without world reads. The **32 cached liquid states** query
the context and take their separate behavior path. The **199 uncached states**
continue through explicit behavior implementations.

`python3 tools/test_contextual_collision_parity.py` validates integrated dispatch
for all **35,723 states**, **11 contexts** and both fluid-flag inputs:
**785,906 fixtures** and **2,391,334 matching output lines per JAR** in the
[contextual-registry report](../benchmarks/rust-contextual-collision-2026-10-06/scoped-parity.json).
Outputs compare cache-relative shape equality or full uncached geometry, plus
ordered world/entity callbacks. This integration corpus supplies missing block
entities; active shulker and piston snapshots retain their separate suites.
These checks establish the recorded collision-query scope, not entity movement
or whole-game parity.


Private shape exports now carry explicit canonical-identity markers. The importer
restores the global empty/full-block shapes only when their complete grid metadata
matches, and rejects duplicate or malformed canonical claims. Equal unmarked
shapes keep distinct identities. Older exports remain readable and report
`has_canonical_identities() == false`; they cannot establish the canonical identity
needed by vanilla's block-collision iterator fast path.

The [canonical-identity report](../benchmarks/rust-canonical-collision-2026-10-06/scoped-parity.json)
adds identity checks for every cached state: **5,566 canonical empty**, **3,348
canonical full-block** and **26,610 other cached states**, with **1,132,980 matching
output lines per JAR**. The integrated contextual comparison also passes again
with **2,391,334 lines per JAR** in its
[regression report](../benchmarks/rust-canonical-collision-2026-10-06/contextual-regression.json).
Five import tests cover legacy data, equal-but-distinct grids, canonical restoration
and malformed identity metadata. Workspace Clippy passes without warnings.


`BlockCollisionScan` now implements lazy cursor selection around entity bounds.
It retains the native epsilon-expanded cursor, X/Y/Z visit order, skipped corners,
large-shape face filtering, moving-piston edge filtering and suffocation-before-
shape query order. Only successful chunk lookups are reused; absent chunks are
queried again. Shape selection uses the global collision world, while suffocation
uses the selected chunk. Canonical full blocks use native AABB intersection;
other shapes use moved voxel intersection. Returned positions are independent
values. Providers supply current block metadata and contextual shape selection.

`python3 tools/test_block_collision_scan_parity.py` compares **4,176 fixtures**
and **517,820 matching lines per JAR** in the
[collision-scan report](../benchmarks/rust-block-collision-scan-2026-10-06/scoped-parity.json).
The native iterator runs over controlled metadata, shapes and chunk availability;
traces cover early termination, missing chunks, query failures, thin/zero-width
bounds, integer-coordinate wrapping and equal shapes with different canonical
identities. Two Rust API tests cover finite/bounded input requirements, cursor
visit budgets and termination after errors. Unsupported cursor volumes return
errors; Java's overflowing iteration counts are outside this bounded API.
Registry/world integration and live movement still need implementation.
