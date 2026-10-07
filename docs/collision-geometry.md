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


`BlockCollisionRegistry` now prepares native cursor-filter flags for every state:
uncached shapes are large, while cached shapes are large when any axis extends
outside the unit block. Moving-piston identity is bound to the same registry.
`SuffocationRegistry` binds the native predicate definitions and resolved
`causes_suffocation` tag. Default predicates combine tag membership with full
collision geometry; overrides preserve piston extension, constant predicates and
lazy shulker block-entity checks. Missing or wrong-type shulker block entities
count as suffocating. Tag changes require rebinding the suffocation registry.

`python3 tools/test_suffocation_parity.py` checks all **35,723 states** with natural,
empty and all-block tag bindings at four positions, including integer extremes.
Every shulker state also covers absent/wrong block entities, all animation states
and query failures. The [suffocation report](../benchmarks/rust-suffocation-2026-10-06/scoped-parity.json)
records **1,697,193 matching lines per JAR**, including scan flags and query traces.
These are registry predicates; live collision-world binding, movement and
suffocation damage remain separate integration work.
The [contextual regression](../benchmarks/rust-suffocation-2026-10-06/contextual-regression.json)
also passes again with **2,391,334 lines per JAR**, and its all-state native audit
now compares large-shape and moving-piston flags. Workspace Clippy passes.


`RegistryCollisionWorld` now binds `BlockCollisionScan` directly to registered
metadata, contextual shapes and suffocation rules. Chunk providers return current
state IDs and a chunk-local block-entity provider; contextual collision uses the
global world. Construction rejects legacy collision exports without complete
canonical identity metadata, because their unit-block fast paths can differ.

`python3 tools/test_registry_collision_scan_parity.py` compares native
`BlockCollisions` over resident `ProtoChunk` sections with the Rust binding over
`TerrainChunk` sections. All registered states cover ordinary and suffocating-only
selection. Additional scenes cover partial/dynamic blocks, chunk and build-height
boundaries, missing chunks, replacements, early termination and query failures.
Shulker fixtures deliberately give the chunk and global providers different
entities to check routing. The [resident collision report](../benchmarks/rust-resident-collision-scan-2026-10-06/scoped-parity.json)
records **3,046,929 matching output lines per JAR**. Six collision import/binding
tests pass, including legacy-metadata rejection; workspace Clippy passes.
Live chunk loading, entity lifecycle, movement and the playable client remain
separate integration work.


`resolve_world_movement` now joins registry-backed block scanning to the native
step-up decision. It selects entity colliders once, including for zero/underflow
movement, then collects applicable border and block shapes in native order for
the initial and optional step query. `try_resolve_step_movement` accepts owned or
borrowed collider lists and propagates provider failures without returning a
partial displacement. The shared implementation avoids allocating reference
lists for owned shapes; existing borrowed callers retain the same API.

`python3 tools/test_world_movement_parity.py` runs actual `Entity.collide` over
registered blocks and resident Java sections, comparing the Rust world collector
and step solver. Every registered state appears as an obstacle; additional cases
cover step heights, grounded/falling motion, ceilings, partial/dynamic blocks,
chunk boundaries, zero/underflow motion and failures during collection. The
[world movement report](../benchmarks/rust-world-movement-2026-10-07/scoped-parity.json)
records **7,062,798 matching lines per JAR**. Entity collider lists, border
proximity/shapes and collision contexts are controlled fixture inputs. Actual
entity selection, border state, travel/gravity/velocity, grounded side effects
and playable integration remain pending.

The [geometry regression](../benchmarks/rust-world-movement-2026-10-07/geometry-regression.json)
passes again with **1,320,187 lines per JAR**, including the existing native step
fixtures. All six movement unit tests and workspace Clippy pass. New tests cover
owned-query failures during both initial and step collection and signed zero
preservation when collection is skipped.


`entity_collision_shapes` now implements `EntityGetter.getEntityCollisions`
selection over ordered entity snapshots: average-size short circuit, epsilon
inflation, exact bounds/excluded-identity filtering, spectator suppression and
lazy collision-policy callbacks. With a source entity it invokes that entity's
`canCollideWith`; without one it invokes each candidate's
`canBeCollidedWith(null)`. Selected bounds become shapes after policy evaluation.
Providers retain responsibility for live storage and each entity type's policy.

`python3 tools/test_entity_collision_parity.py` compares the actual native getter,
default vehicle-root checks and controlled policy overrides, including failures
at query/callback boundaries. Tiny/zero-width and nonfinite query boxes, inflated
boundary contacts, spectators, shared vehicles, null sources and ordered shape
outputs are covered. The [entity collision report](../benchmarks/rust-entity-collision-2026-10-07/scoped-parity.json)
records **158,558 matching lines per JAR**. Workspace Clippy passes. This adds the
selection helper needed by movement; live entity indexing, lifecycle and
entity-type policy implementations remain pending.


`BorderGeometry` now models stationary and moving world-border extents, cached
stationary collision geometry, center/absolute-limit changes, tick progression,
partial-tick bounds and movement proximity. The pinned version decrements size
transition progress once per tick. Ordinary bounds and collision shapes use the
previous size while moving; completing the transition installs a stationary
extent at the target. Collision geometry subtracts the floor/ceil-rounded inner
column from the infinite outer shape. Boundary predicates retain their asymmetric
inclusive minimum/exclusive maximum tests and native floating-point order.

`python3 tools/test_world_border_parity.py` compares actual `WorldBorder` mutation
sequences, extent metadata, complete shape geometry and proximity predicates.
The [border report](../benchmarks/rust-world-border-2026-10-07/scoped-parity.json)
records **1,138,830 matching lines per JAR**, including zero/large durations,
wrapping start timestamps, completion, retargeting, signed zero, extreme finite
centers/sizes and nonfinite query/interpolation inputs. The supported construction
and mutation domain requires finite nonnegative sizes, finite centers and
nonnegative absolute limits/durations. A unit test verifies invalid parameters
leave the existing extent unchanged; workspace Clippy passes. Saved-data codecs,
listeners, dirty-state events, damage, synchronization and live movement binding
remain separate work.


`restitute_movement` now implements the pinned post-collision velocity calculation
with lazy entity/block bounce queries, gravity thresholds, nonliving float
scaling and air-drag interpolation. It preserves horizontal/vertical collision
branches, callback ordering, the position-sync field write and final velocity
assignment. Errors retain already performed effects instead of rolling back the
world or returning a fabricated successful velocity.

`python3 tools/test_restitution_parity.py` executes actual
`Entity.restituteMovementAfterCollisions` on living and nonliving snapshots with
controlled block restitution and suppression tags. The
[restitution report](../benchmarks/rust-restitution-2026-10-07/scoped-parity.json)
records **177,090 matching lines per JAR**. Fixtures cover changing lazy getter
values, signed zero/nonfinite arithmetic, bounce callbacks that alter velocity,
and failures before/after each side effect. Workspace Clippy passes. Registered
block property/tag provisioning, actual bounce sounds/events, `Entity.move`
guards, fall damage, grounding and full player simulation remain pending.


`find_supporting_block` now selects the nearest colliding block center using the
native distance arithmetic and wrapping Y/Z/X position tie-break. `GroundState`
implements the thin foot query, previous-horizontal-footprint fallback and both
ground/collision flag updates. Null movement preserves the prior support in the
native branch where no fallback query can run; query failures retain preceding
flag writes.

`python3 tools/test_ground_support_parity.py` compares actual
`CollisionGetter.findSupportingBlock`, `Entity.checkSupportingBlock` and both
`setOnGroundWithMovement` overloads. The
[ground support report](../benchmarks/rust-ground-support-2026-10-07/scoped-parity.json)
records **342,709 selection lines and 151,444 state-transition lines per JAR**,
covering ties, nonfinite entity positions, missing chunks, query errors, nullable
movement and sequential changes of support. Shape/context snapshots are controlled;
registry-backed scanning has separate integration coverage. Workspace Clippy
passes. Full live entity movement, support-dependent effects and gameplay remain
pending.
The [shared scanner regression](../benchmarks/rust-ground-support-2026-10-07/scan-regression.json)
also passes with **517,820 matching lines per JAR** after extending its fixtures.


`movement_support` now implements support-dependent positions, the three public
offset variants and base-entity block jump/speed factors. It preserves the support
X/Z coordinates, conditional fence/wall/gate lookup, float-to-double offset
widening, cached entity block position and lazy block queries. Jump-factor lookup
always reads the lower block; speed lookup can short-circuit for water, bubble
columns or a non-unit current factor.

`python3 tools/test_movement_support_parity.py` compares the actual native methods
with controlled state/tag/property snapshots. The
[movement support report](../benchmarks/rust-movement-support-2026-10-07/scoped-parity.json)
records **51,572 matching lines per JAR**, including float boundaries, nonfinite
positions/offsets/factors, tag/type short circuits and errors at every query.
Workspace Clippy passes. Registered movement-property provisioning, living-entity
efficiency and the full movement lifecycle remain pending.


`MovementProperties` imports version-bound raw friction, speed, jump, bounce and
fall-distance-reduction fields plus fence-gate type. `BoundMovementProperties`
binds resolved fence/wall/bounce-suppression tags; rebinding leaves previous
snapshots unchanged. `RegistryMovementWorld` connects real state IDs from a block
provider to the support-position and factor calculations, rejecting unknown IDs.

`python3 tools/test_movement_registry_parity.py` independently checks native
property getters for every state, then runs actual entity support/factor queries
over all **35,723 states / 1,286 blocks** under normal, empty and universal tags.
Mixed-block scenes and failing providers check query order. The
[registered movement report](../benchmarks/rust-movement-registry-2026-10-07/scoped-parity.json)
records **2,300,888 matching lines per JAR**. Three import/binding tests and
workspace Clippy pass. This provisions movement data; full player travel, live
entity state and block bounce effects remain separate integration work.


`PistonMovement` now accumulates displacement per axis and resets it when the
game tick changes. It preserves the initial squared-length short circuit, lazy
time lookup, first nonzero axis, the +/-0.51 cumulative clamp and float-derived
small-displacement threshold. A failed time lookup leaves the accumulator intact.

`python3 tools/test_piston_movement_parity.py` runs actual
`Entity.limitPistonMovement` across sequential tick/push inputs. The
[piston movement report](../benchmarks/rust-piston-movement-2026-10-07/scoped-parity.json)
records **325,197 matching lines per JAR**, including repeated/opposing pushes,
threshold neighbors, signed zero, nonfinite inputs and injected failures.
Workspace Clippy passes. Piston block events, entity pushing and the surrounding
`Entity.move` lifecycle remain pending.
The [registered movement regression](../benchmarks/rust-piston-movement-2026-10-07/movement-registry-regression.json)
passes again with **2,300,888 lines per JAR** on the integrated sources.


`no_world_collision` implements the entity-bearing collision-free query over the
registry-bound world. It consumes block collisions lazily, then checks whether
the selected entity list is empty, then tests applicable border geometry. Each
stage short-circuits the later providers; a nonempty entity list blocks the query
even if its supplied shape is empty. Collision contexts are supplied explicitly.

`python3 tools/test_world_collision_check_parity.py` executes native
`CollisionGetter.noCollision` over registered blocks and resident Java sections.
The [world collision-check report](../benchmarks/rust-world-collision-check-2026-10-07/scoped-parity.json)
records **4,793,095 matching lines per JAR**. All states, context variants, empty
and disjoint controlled entity/border shapes, missing chunks, boundaries and
provider failures are covered. Workspace Clippy passes. Null-entity overloads,
actual entity-list/border providers and live player ledge behavior remain pending.
The [shared movement regression](../benchmarks/rust-world-collision-check-2026-10-07/movement-regression.json)
also passes with **7,062,798 matching lines per JAR**.


`back_off_from_edge` implements the player ledge guards, above-ground test and
X/Z/diagonal displacement reductions. It retains lazy step/staying/grounded
queries, float step widening, signed-zero behavior and the exact thin collision
probe arithmetic. A caller-supplied query budget bounds nonconverging inputs and
returns an error without fabricating a successful displacement.

`python3 tools/test_edge_backoff_parity.py` executes actual player methods with
controlled collision-free query results. The
[edge backoff report](../benchmarks/rust-edge-backoff-2026-10-07/scoped-parity.json)
records **46,578 matching lines per JAR**, covering all mover types, flight/crouch
guards, fall/step and 0.05 thresholds, nonfinite values and query failures. Two
query-budget unit tests and workspace Clippy pass. Registered-world ledge probes,
live crouching/attribute state and the complete player movement loop are separate
integration work.


Registered-world ledge integration now runs `back_off_from_edge` through
`no_world_collision` and registry-bound resident chunk scans. The native side
executes actual `Player.maybeBackOffFromEdge` and its collision probes over
`ProtoChunk` sections. Every state is used as a supporting block, with additional
falling, step-height, chunk-edge, diagonal, dynamic-block and guard/error cases.

`python3 tools/test_world_edge_backoff_parity.py` passes with
**27,109,699 matching lines per JAR** in the
[world ledge report](../benchmarks/rust-world-edge-backoff-2026-10-07/scoped-parity.json).
The shared fixture now supports player probes alongside direct movement and
collision checks; workspace Clippy passes. Entity lists, border applicability and
collision contexts remain controlled snapshots. A complete player movement/travel
loop and playable client remain pending.


`check_fall_damage` now implements base-entity fall-distance accumulation and
landing order. Downward displacement narrows to float before being subtracted
from the double fall distance. Grounded landings call the block, read the
possibly changed position/support, emit the ground event and reset fall distance.
Errors preserve completed field writes and callback effects.

`python3 tools/test_fall_damage_parity.py` runs actual `Entity.checkFallDamage`
with controlled water, landing and event providers. The
[base fall report](../benchmarks/rust-fall-damage-2026-10-07/scoped-parity.json)
records **44,566 matching lines per JAR**, including nonfinite arithmetic, float
underflow, callback mutations and failures at every boundary. Workspace Clippy
passes. Living-entity fluid/particle behavior, registered block landing effects,
health/damage and full movement remain pending.


`living_fall` implements default living-entity fall power/damage calculation,
impulse-height exemption and landing damage callback order. It retains separate
immunity checks for passenger propagation and damage calculation, double/float
arithmetic and saturating floor-to-int conversion. Positive damage resets the
impulse context, selects the fall sound, plays block landing sound and dispatches
hurt. A rejected hurt callback does not change the native positive-damage result.
`ImpulseContext` also models setter, grace-extension and reset behavior; disabling
ignore clears grace time while retaining the impact position until a later reset.

`python3 tools/test_living_fall_parity.py` compares actual living methods and
sequential impulse changes. The
[living fall report](../benchmarks/rust-living-fall-2026-10-07/scoped-parity.json)
records **153,025 matching lines per JAR**, including changing immunity, passenger
callback mutations, nonfinite arithmetic, sound threshold selection and failures
after partial state changes. Workspace Clippy passes. Attribute provisioning,
passenger storage/recursion, actual health processing, tick/save/network behavior
and the complete movement lifecycle remain pending.


`push_entities_up` implements the block-replacement push: query old/new shapes,
subtract old occupied space without optimizing the join, translate to world
coordinates and select entities using the added shape's bounds. It reads each
current entity box before calculating its upward displacement and invokes a
relative teleport even when that displacement is zero.

`python3 tools/test_block_push_parity.py` compares actual `Block.pushEntitiesUp`
with controlled shapes and selected entity lists. The
[block push report](../benchmarks/rust-block-push-2026-10-07/scoped-parity.json)
records **101,006 matching lines per JAR**, including empty/redundant geometry,
partial shapes, large coordinates, nonfinite entity boxes, mutations of later
entities and failures after partial effects. Workspace Clippy passes. Registered
replacement dispatch, actual entity selection and teleport/network semantics
remain separate integration work.


`block_landing` binds audited native landing/bounce families to every registered
state. It implements fall-distance reduction, hay/honey damage multipliers,
slime suppression, powder-snow fall sounds, upward dripstone tips, farmland
replacement with occupant pushes, turtle-egg destruction/decrement and shelf
mushroom bounce sounds. Random draws, server-class checks, client flags, spawn
protection, mob griefing, sounds, writes and events retain native order.

`python3 tools/test_block_landing_parity.py` invokes registered native methods for
all **35,723 states / 1,286 blocks** with eight entity class families. The
[registered landing report](../benchmarks/rust-block-landing-2026-10-07/scoped-parity.json)
records **3,524,267 matching lines per JAR**, including float/double boundaries,
NaN/infinity, failed writes, callback failures, egg hatch preservation and
farmland collision pushes. Targeted Clippy passes. Actual health, live entity
selection/storage, teleport/network behavior, audio playback and full movement
lifecycle integration remain separate work.


`movement_emission` accumulates native movement/flight distances, evaluates
climbable and powder-snow states, and dispatches step/swim/flap effects. Legacy
and current support blocks retain their separate sound/event rules. Threshold
updates use Java float-to-int saturation and wrapping integer addition, and
sound/event callbacks can change fields observed by subsequent operations.

`python3 tools/test_movement_emission_parity.py` compares actual native movement
emission, block vibration dispatch, flapping and next-step arithmetic. The
[movement emission report](../benchmarks/rust-movement-emission-2026-10-07/scoped-parity.json)
records **598,611 matching lines per JAR** across **53,036 fixtures**, including
predicate combinations, sound/event modes, differing support blocks, nonfinite
numbers, mutations and provider failures. Targeted Clippy passes. Sound callbacks
are controlled and the crystal-sound predicate is false in this gate; registered
sound selection, audio playback and full movement integration remain pending.


`entity_move` sequences native movement phases: no-physics/piston early exits,
stuck multiplier reset, edge backoff, collision resolution, fall-reset clipping,
position/recording writes, collision/ground flags, landing, removal, restitution,
movement emission and block speed multiplication. `MovementHistory` retains at
most 100 records by merging the oldest pair and dropping their original-axis
movement metadata. Callbacks can change fields read by later phases.

`python3 tools/test_entity_move_parity.py` invokes actual `Entity.move` with native
shape resolution and zero restitution. The
[movement lifecycle report](../benchmarks/rust-entity-move-2026-10-07/scoped-parity.json)
records **543,653 matching lines per JAR**, covering signed zero, tiny/nonfinite
movement, collisions, early exits, recording cap, authority guards, landing
mutations/removal and failures after partial writes. Targeted Clippy passes.
This gate controls piston/edge decisions, ground support, ray hits and landing
callbacks; step height is zero and non-passenger emission is tested separately.
Binding all phases to a live entity/world and player travel remains pending.


`world_ray` implements ordered block/fluid ray clipping, nearest-hit selection
with block precedence on ties, and interaction-shape overrides that replace only
the hit face. It also implements native fluid pick modes and fall-reset shape
selection, including player type identity, end portals/gateways and server-side
zero-delay Nether portals. Traversal limits and invalid geometry return errors.

`python3 tools/test_world_ray_parity.py` compares actual `BlockGetter.clip` and
native `ClipContext` modes. The
[world ray report](../benchmarks/rust-world-ray-2026-10-07/scoped-parity.json)
records **248,725 matching lines per JAR**, including inside hits, ties, misses,
interaction geometry, short/signed-zero rays, lazy predicates/rules and provider
failures. Targeted Clippy passes. Separate limit tests distinguish a miss from
nonfinite geometry, exhausted traversal budget and failed world reads. Shapes
and state predicates are controlled; registered shape binding, actual fluid
shape caching and full entity/world integration remain pending.


`FluidShapeCache` matches native registered-fluid shape caching. The first
successful lookup of a nonempty fluid state samples the fluid above and retains
its resulting shape across later positions/world views. Failed reads leave the
state uncached. Empty and full cube shapes preserve canonical identity; distinct
noncanonical cached shapes retain their own identities.

`python3 tools/test_fluid_shape_parity.py` runs every registered native fluid state
in changing query sequences. The
[fluid shape report](../benchmarks/rust-fluid-shape-2026-10-07/scoped-parity.json)
records **387,950 matching lines per JAR** across **5,181 sequences**, comparing
geometry, cache population, canonical/alias identities, query suppression and
failures. Targeted Clippy passes. The pinned registry has no amount-nine state;
unsupported custom fluid definitions are outside this module. Fluid propagation,
entity immersion and live world/ray integration remain pending.


`InteractionShapes` imports a complete state-bound table with a distinct table
kind and mandatory canonical identity metadata. The pinned owner audit finds
only default empty, cauldron, composter, hopper and scaffolding implementations;
all are static per state. Geometry uses the existing validated discrete shape
importer while preserving eight native shape identities.

`python3 tools/test_interaction_shape_parity.py` compares every registered state
and actual native interaction-face overrides with supplied collision geometry.
The [interaction shape report](../benchmarks/rust-interaction-shape-2026-10-07/scoped-parity.json)
records **188,682 matching lines per JAR** across all **35,723 states**. Native
interaction-shape world queries are rejected by the fixture. Import checks reject
wrong table kind, partial mappings, missing canonical metadata, foreign state
ordering and invalid IDs. Targeted Clippy passes. Registered outline/visual
shapes, complete ray-world binding and gameplay integration remain pending.


`FallRayRegistry` binds fall-reset tags, portal identities, interaction geometry
and fluid states to the complete registered block-state table. `fall_reset_ray`
uses that binding with the shared fluid-shape cache and native water selection.
Tag reloads produce new validated registry snapshots; unknown tag members and
invalid state IDs are rejected.

`python3 tools/test_registry_fall_ray_parity.py` compares actual registered
`BlockGetter.clip` behavior with native block/fluid tags and shape getters. The
[registered fall-ray report](../benchmarks/rust-registry-fall-ray-2026-10-07/scoped-parity.json)
records **3,024,623 matching lines per JAR** across **120,297 fixtures**, including
all **35,723 states**, tag reloads, server/client distinctions, player type checks,
portal delays, changing above-fluid states, shared cache reuse and failures.
Targeted Clippy passes. World storage is controlled; live entity movement and
complete player/world gameplay integration remain pending.


`living_landing` wraps base fall handling with fluid refresh, server-only changed
block notification and landing particles, followed by climbing-position cleanup.
Particle positions preserve the native support-block edge projection and capture
entity coordinates before later callbacks. Attribute, fluid, particle and block
callbacks can mutate fields; failures retain preceding writes.

`python3 tools/test_living_landing_parity.py` invokes actual
`LivingEntity.checkFallDamage` and its base implementation. The
[living landing report](../benchmarks/rust-living-landing-2026-10-07/scoped-parity.json)
records **244,844 matching lines per JAR**, including server/nonserver levels,
water changes, numeric boundaries, particle locations/counts, callback failures,
field mutations and climbing cleanup. Targeted Clippy passes. Fluid immersion,
changed-block enchantments, particle rendering and live gameplay binding remain
pending; those providers are controlled by this gate.


`living_air` implements input normalization/rotation, friction-influenced speed,
ladder/scaffolding limits, relative movement, collision-triggered climbing and
air travel. Ground friction is captured before movement while levitation,
gravity and drag are sampled afterwards. Player class identity controls ladder
slide suppression. Native float arithmetic and quantized trigonometry are kept.

`python3 tools/test_living_air_parity.py` invokes the actual native private air,
relative movement, climbing, speed, friction and input-vector methods. The
[air travel report](../benchmarks/rust-living-air-2026-10-07/scoped-parity.json)
records **443,554 matching lines per JAR**, including both player/nonplayer
classes, changed ground/climbing predicates, missing client chunks, levitation,
nonfinite boundaries, callback mutations and failures. Targeted Clippy passes.
Extreme effect amplifiers are injected directly for integer-boundary checks;
normal constructor clamping is outside this gate. Collision movement, gravity,
attributes and immersion remain controlled providers; live gameplay binding and
complete player travel are pending.


`living_fluid` adds native water/lava acceleration, slowdown, falling adjustment,
shallow-lava handling, collision escape and ridden buoyancy. It shares input
rotation with air movement. Fluid heights and escape queries remain lazy, with
movement mutations visible to subsequent velocity updates.

`python3 tools/test_living_fluid_parity.py` compares the actual native travel and
fluid helper methods. The
[fluid travel report](../benchmarks/rust-living-fluid-2026-10-07/scoped-parity.json)
records **265,100 matching lines per JAR**, including sprint changes, movement
efficiency, dolphin effects, shallow-fluid boundaries, escape probes, ridden
float guards, nonfinite values, callback mutations and failures. Targeted Clippy
passes. Collision movement, immersion depths, free-space queries, attributes and
gravity are controlled; live fluid propagation and gameplay binding remain
pending.


`living_motion` implements default living jump strength/boost, sprint jump
impulses, gravity and slow falling, vertical liquid input, rider flight speed
and air drag. Native finite-addition filtering and delayed synchronization flags
preserve partial writes when providers fail.

`python3 tools/test_living_motion_parity.py` compares the corresponding actual
living methods plus `Entity.applyGravity` and `addDeltaMovement`. The
[jump and gravity report](../benchmarks/rust-living-motion-2026-10-07/scoped-parity.json)
records **113,623 matching lines per JAR**, including float thresholds, extreme
raw effect amplifiers, signed zero/nonfinite values, rider classes, callback
mutations and failures. Targeted Clippy passes. Entity-type jump/flight overrides,
effect and attribute storage, registered jump-factor binding and live gameplay
integration remain pending.


`player_travel` wraps living travel with passenger bypass, swimming pitch
adjustment, head-fluid checks and preservation of creative-flight vertical
velocity. It also binds player flight speed to sprint/passenger/ability rules
and implements the flying ability's fluid-affection predicate.

`python3 tools/test_player_travel_parity.py` invokes actual `Player.travel`,
`getFlyingSpeed` and `isAffectedByFluids`. The
[player travel report](../benchmarks/rust-player-travel-2026-10-07/scoped-parity.json)
records **120,933 matching lines per JAR**, including ability and velocity
mutations, callback failures, signed zero/nonfinite positions and the distinction
between direct ability fields and virtual ability access. Targeted Clippy passes.
Parent travel is selected through a controlled fluid callback; connecting the
verified living phases and collision movement to the live player/world remains
pending.


`ClimbingRegistry` binds resolved climbing/glide-through tags and actual trapdoor
class identities to all registered states. `on_climbable` preserves spectator
and gliding guards, open-trapdoor/ladder facing checks, wrapping below-position
arithmetic and last-climbable position writes. Negative or failed checks retain
the previous climbing position. Unknown members/properties and IDs are rejected.

`python3 tools/test_climbing_parity.py` invokes actual registered
`LivingEntity.onClimbable` and trapdoor support checks. The
[climbing report](../benchmarks/rust-climbing-2026-10-07/scoped-parity.json)
records **3,820,054 matching lines per JAR** across all **35,723 states**, including
all trapdoor states against every registered ladder facing, tag reloads,
spectator/gliding guards, captured positions, mutations and failures. Targeted
Clippy passes. Entity/world storage is controlled; live movement binding and
equipment-based glide eligibility remain pending.


`fluid_occupancy` implements section-ordered liquid checks over resident block
sections with a complete registered fluid-presence table. `BlockSection.any_state`
checks active palette entries so dry sections skip cell unpacking. Bounds use
inclusive floored endpoints, and chunk requests retain native behavior even when
no requested vertical section is within world height. Invalid geometry, foreign
registries and query-budget exhaustion return explicit errors.

`python3 tools/test_fluid_occupancy_parity.py` compares actual
`LevelReader.containsAnyLiquid`, `BoxBlockMatcher` and `BlockScanUtils` with
native ProtoChunk sections. The
[liquid occupancy report](../benchmarks/rust-fluid-occupancy-2026-10-07/scoped-parity.json)
records **116,664 matching lines per JAR**, including fluid presence for all
**35,723 states**, public chunk/section query order, height boundaries, palette
changes/compaction and failures. Rust-only checks verify typed budget, nonfinite
and registry errors. Targeted Clippy passes. Chunk snapshots repeat controlled
resident data; collision composition for `Entity.isFree` and live world binding
remain pending. No performance gain is claimed without measurement.


`free_space.offset_is_free` composes native offset bounds, registry-bound
block/entity/border collision clearance and resident liquid occupancy. The
liquid scan runs only after collision clearance succeeds. Both phases share
resident TerrainChunk storage through `RegistryCollisionWorld`; their typed
errors and separate query budgets remain visible to the caller.

`python3 tools/test_free_space_parity.py` invokes actual `Entity.isFree`, native
collision iteration and liquid scanning on the corresponding resident ProtoChunk
storage. The [free-space report](../benchmarks/rust-free-space-2026-10-07/scoped-parity.json)
records **5,764,046 matching lines per JAR** across all **35,723 states**, offsets,
fluid/context variants, height boundaries, required missing chunks and failures.
Targeted Clippy passes. Entity lists, border applicability and collision contexts
are controlled; chunk generation, live entity indexing and full playable
integration remain pending.
The [collision-only regression](../benchmarks/rust-free-space-2026-10-07/collision-regression.json)
also passes **4,793,095 matching lines per JAR** after extending the shared
fixture and registry binding.
