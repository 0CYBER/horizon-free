# Horizon Rust rewrite status

The behavior target is the repository's pinned **Minecraft Java Edition 26.3**.
One-to-one vanilla gameplay parity is the acceptance criterion. It has **not**
been reached. Horizon now has a [native development client](native-client.md)
that renders a controlled block scene and twelve baked resource models with a
free camera. It does not run a
complete game server or gameplay simulation.
The Rust workspace implements core arithmetic/random streams, world data and
resident sections, scheduled tick ordering, geometry, contained fluid states,
noise/density evaluation, biome and terrain generation kernels, material rules,
and parts of feature placement and decoration. All pinned block-survival
families now have native differential coverage. Full simulation callbacks,
complete chunk generation, entity behavior, networking, rendering and playable
integration remain unfinished. The linked subsystem documents and hashed reports
record the exact tested scope and remaining gaps.
The `horizon-assets` crate now decodes pinned cuboid models, with all 2,883 bundled
block-model JSON files and 2,457 synthetic cases compared against the actual
client parser. [Decoder scope and validation](native-client.md#validation)
also include parent/texture resolution over 5,648 roots and cuboid baking over
7,100 roots with identity blockstate transforms. Blockstate selection, transformed
models and UV locking remain unfinished. Blockstate resource decoding now
matches all 1,288 bundled definitions and 1,351 synthetic cases; registry
binding and model selection are not yet connected to the renderer. The native renderer now draws an
inspection gallery from the decoded/resolved/baked models, including binary
cutouts and stepped lantern animation; these are not yet gameplay blocks.
Animation metadata, frame sequences and tick-state selection now have native
differential coverage, including all bundled animated texture metadata. GPU
interpolation and game-loop synchronization remain unfinished.
The playable Java performance packs remain available during development.

## Completion criteria and playable builds

The final deliverable is the complete Horizon Rust game with one-to-one parity
with pinned Java Edition 26.3, plus playable builds for **macOS, Linux and
Windows** that the user can download and test. Each platform must include a
packaged native executable, required runtime resources, launch instructions,
artifact checksums, and the tested OS/CPU/GPU configuration. The existing M2 Pro
Mac is the first physical macOS target.

A platform is complete only after its packaged Rust game is launched and tested:
create/load a world, render terrain, use keyboard/mouse controls, move and interact,
break/place blocks, use inventory/crafting, save, quit and reload. These launch
checks accompany the full gameplay-parity suite; they do not replace it. Record
platform-specific failures and test limits. Compiler success, library comparisons
and the existing Java packs do not establish a playable Rust build.

Optimize FPS, frame pacing, memory use and render distance using matched scenes,
worlds, settings and hardware after behavior is validated. Package and test each
supported platform before declaring the rewrite complete. Commit and push
verified development checkpoints regularly.

## Implemented foundation

`crates/horizon-core` is a dependency-free Rust library with no unsafe code.
It uses small integer position values, exclusively owned RNG state, and packed
64-bit storage. These choices avoid Java object overhead in this slice; no
whole-game memory, FPS, or render-distance gain has been measured.

| Area | Implemented and checked operations | Remaining scope |
| --- | --- | --- |
| Positions | Block, section, and chunk packing/unpacking; wrapping block offsets; block-to-section/chunk conversion; section-relative packet positions; block-state storage indices; region coordinates/local indices; bounded inclusive/directional block-box iteration | Remaining iterators, world bounds and entity/world integration |
| Simulation geometry | Selected Vec3 operations, quantized trig/rotations, normalized AABB bounds and transforms, finite block traversal/swept visitation, ray faces, immutable cube/array voxel grids, greedy boxes, collision distances, column heights, closest points, lazy floor height, box quantization and Boolean/optimized joins; movement resolution over supplied shapes, step candidates and step-up decisions/query bounds | Shape rotations/faces/edges, entity-context shape selection, full entity movement and side effects |
| Legacy random stream | Seeding/reseeding; int, bounded/inclusive/exclusive int, long, boolean, float, double; triangular float/double draws; fork; consume count; positional factories and Java UTF-16 string hashes | Gaussian sampling, threading-error behavior and remaining world-generation call sites |
| Xoroshiro random stream | 64-bit seed expansion; explicit 128-bit state including all-zero fallback; the same selected random operations as above; positional factories and MD5-derived seeds with Java UTF-8 surrogate handling | Gaussian sampling, serialization and remaining world-generation call sites |
| Noise generation | 26.3 Perlin/simplex/smeared Perlin; permutation construction and coordinate wrap; Perlin derivatives; cached/scalar density-volume accumulation; nested/flattened stacks; normal-noise octave weights and normalization; modern and legacy Nether seeding | Serialized noise definitions, density graphs, complete chunk generation, biome/feature/structure integration |
| Density evaluation | Compiled constants, clamped/repeating/mirrored gradients, unshifted and XZ/XYZ shifted noise; offset-noise variants, old blended noise, outer End islands and distance-to-point; all eleven unary operators, six binary operators, powers, rounding, constant-operand specializations, min/max, clamp, range choice, interval selection and linear interpolation; separate scalar/bulk paths, nested scratch buffers, context-bound blend alpha/offset/structure samplers, absent/empty blend-density wrappers, context-owned scalar/volume caches, coordinate slices, coarse-grid interpolation, surface searches and nested cubic splines | Codecs, structure-density producers, nonempty old-world blending and terrain integration |
| Density graph compilation | Immutable definitions for constants, gradients, arithmetic/selection operations, powers, rounding, distance-to-point, slices, caches, interpolation, surface search, nested splines, outer End islands and normal/offset/blended noise; context fields, blend-density wrappers, direct/named reference holders; range/axis metadata, constant and disjoint-range specialization, uniform-axis rewriting, cache-ID allocation/deduplication, memoized roots, spline coordinate sharing/validation, world-seed lookup and failure state | Codecs, concrete world-field producers and complete terrain integration |
| Density range analysis | Checked interval constructors, hulls, arithmetic/min/max, reciprocal, abs/square/sign, powers/logarithms, clamp, monotonic endpoint mapping, finite/infinite interpolation, predicates, equality and Java hashes; conservative spline range propagation; integration with supported graph definitions | Propagation through remaining graph definitions |
| Packed palette-index storage | Widths 1–32; allocation; imported raw words; reads; writes returning old values; unpack; fill; padding/tail preservation; invalid-write rejection | Remaining network container integration |
| World data | Named/network NBT, modified UTF-8, mixed lists; gzip/zlib/uncompressed/lz4-java codecs; region exports including external chunks; block/biome saved palettes | Complete save lifecycle, data fixing, concurrent saves and recovery |
| Resident block sections | Pinned block-state registry import, defaults, numeric IDs, uniform sections, packed palette growth, freed-slot reuse and compaction; cached collision shape lookup bound to the same registry | Block behavior, entity-context collision shapes, light and full chunk-lifecycle integration |
| Heightmaps | Six type predicates and flags, packed heights, updates/removal rescans, shared priming, raw imports and length-mismatch fallback; all pinned states and mutation fixtures compared | Block air/fluid behavior and chunk-lifecycle integration |
| Scheduled ticks | Chunk queues, level collection/drain ordering, duplicate suppression, seven priorities, saved delays, chunk eligibility, callback mutations, area removal/copying, saved block/fluid NBT | Actual block/fluid callbacks, neighbor updates, tick-time and chunk-lifecycle integration |

Packet-relative positions use X/Z/Y ordering; block-state storage indices use
Y/Z/X ordering. Negative coordinate conversion must floor rather than truncate.
The legacy RNG must retain Java's signed-int arithmetic and rejection rules;
the xoroshiro RNG has a different bounded-int algorithm and bit selection.
Positional factories preserve signed coordinate hashing and each stream's distinct
seed derivation. The hash API accepts UTF-16 units so malformed surrogate sequences
retain Java's replacement behavior. Gaussian sampling is still absent: a private
probe found bit differences between Java `Math.log` and the platform logarithm,
which must be resolved before its values and cached-draw behavior can be claimed.
Storage entries never cross a word boundary. Vanilla's fill operation also
writes padding and unused tail bits; imported words retain their original bits
when a slot is changed. These details are covered by the differential corpus.

Rust APIs return typed errors instead of Java exceptions. Storage lengths and
indices use `usize`; raw decoded values use `u32` bit patterns. Writes reject
values above the nonnegative Java-int domain, including at width 32. Test output
casts raw imported width-32 reads back to signed Java ints for comparison.
Negative lengths and the complete Java object/API surface are not supported.

## Reproduce validation

The core/world libraries require Rust 1.85 or newer; the native client's locked
dependencies require Rust 1.90 or newer. Use current stable Rust, Python 3.10 or
newer, and JDK 25 for the complete workspace. From the checkout:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 tools/test_rust_parity.py --prepare
python3 tools/test_world_parity.py
python3 tools/test_tick_parity.py
python3 tools/test_geometry_parity.py
python3 tools/test_collision_parity.py
python3 tools/test_noise_parity.py
python3 tools/test_density_parity.py
python3 tools/test_worldgen_parity.py
python3 tools/test_heightmap_parity.py
python3 tools/test_terrain_parity.py
python3 tools/test_structure_density_parity.py
python3 tools/test_material_surface_parity.py
python3 tools/test_material_condition_parity.py
python3 tools/test_material_stage_parity.py
python3 tools/test_biome_zoom_parity.py
python3 tools/test_biome_chunk_parity.py
python3 tools/test_biome_generation_parity.py
python3 tools/test_biome_source_parity.py
python3 tools/test_anchor_parity.py
python3 tools/test_biome_temperature_parity.py
python3 tools/test_float_log_parity.py
```

`--prepare` regenerates the local reference artifacts from the existing lock;
omit it after preparation to run without downloads. The runner verifies the
official manifest and both game downloads, the server's inner game JAR, and each
server dependency against pinned/bundled hashes. The original Java test adapter
calls vanilla classes directly; it does not substitute a Java reimplementation
of the Rust algorithms. The same corpus runs against client and server code.
The fixtures need no display, account, assets, or world generation.

The deterministic corpus contains coordinate extremes, all 4,096 local section
positions, 8,192 random coordinate/offset queries, boundary and random signed
seeds, mixed RNG operations, fork/reseed/consume checks, invalid bounds, and
all 32 storage widths at boundary and 4,096-entry sizes. It also imports words
containing all set bits and compares the raw padding and fill results. The
current run produces **1,634,520 matching output lines per reference JAR**.
It includes 4,400 positional-factory commands and 5,000 random-helper commands,
with overflowing integer intervals, hash padding boundaries and malformed UTF-16.
This count includes repeated checks and raw entries; it is not a count of
distinct vanilla behaviors or a percentage of game coverage.

Artifacts remain gitignored under `.local/rust-parity/`: corpus, Rust output,
client/server output, compiled oracle, and `report.json`. The report records
source/input/output hashes, reference identities, tool versions, and matched
line counts. A prior success report is removed before a new run can fail.
CI runs the same comparison in addition to offline Rust and Python checks.
The checked-in [scoped validation report](../benchmarks/rust-core-2026-10-05/scoped-parity.json)
contains only metadata and digests from the local run, not game code or artifacts.

The additional [world-data suite](world-data.md) checks NBT and region
interoperability, saved palettes, all 35,723 registered block-state IDs, and
resident sections after repeated mutation. The corresponding
[scoped report](../benchmarks/rust-world-2026-10-05/scoped-parity.json)
records client/server reference hashes and the tested source hashes.

The [scheduled-tick suite](scheduled-ticks.md) adds 25,815 commands including
2,096 tick invocations, producing 97,776 matching output lines per reference.
It also verifies block/fluid tick save codecs and a persisted region chunk.
The [geometry and cached-collision suites](collision-geometry.md) check numerical
boundaries and runtime block-state shape data, with separate
[geometry](../benchmarks/rust-geometry-2026-10-05/scoped-parity.json) and
[cached-collision](../benchmarks/rust-collision-2026-10-05/scoped-parity.json)
metadata reports. All differential runners reject changed source hashes during a run before publishing
a success report. Historic reports describe their own recorded source versions.

The [noise suite](noise-generation.md) checks base sampling, derivative and bulk
arithmetic, nested/flattened stacks, normal-noise normalization and both modern
and legacy Nether initialization. Its [scoped report](../benchmarks/rust-noise-2026-10-05/scoped-parity.json)
records the tested sources and both reference artifacts.

The [density suite](density-functions.md) checks compiled scalar and bulk samplers,
including gradients, nested shifted noise, arithmetic, caching, axis slicing and
coarse-grid interpolation, surface searches and nested splines with range analysis;
it also compares the initial graph compiler with vanilla's compiler. Its
[scoped report](../benchmarks/rust-density-2026-10-05/scoped-parity.json) records
**15,524,627 matching output lines per reference JAR**. That suite constructs graphs through its transport. Serialized registry
comparisons are covered separately below; generated terrain remains pending.

The [float-log report](../benchmarks/rust-float-log-2026-10-05/scoped-parity.json)
records exhaustive compiled scalar comparisons for all **4,294,967,296 float bit
patterns** against both JARs on the local Linux/JDK runtime, with canonical NaN
results. Double-precision logarithms and other-platform math remain separate
validation work.

The [world-generation codec suite](world-generation.md) checks every bundled
normal-noise JSON definition and all 55 bundled density definitions against
vanilla's actual registry-aware codecs. It compares decoded octave metadata,
graph ranges and axes, raw and optimized compilation, cache allocation and
seeded scalar and strided bulk sampling. Its
[scoped report](../benchmarks/rust-worldgen-2026-10-06/scoped-parity.json) records
**961,693 matching output lines per reference JAR**. All seven bundled eight-field
routers and their climate samplers are also
compared, including quart-coordinate wrapping and float/long quantization.
Climate search-tree construction and warm/fresh ties are checked across the
bounded API. Scalar and prepared-volume multi-noise biome resolvers are implemented
using private imported built-in parameter tables and compared against vanilla
scalar and bulk biome resolvers. Climate parameter codecs, vertical terrain bounds and the global fluid picker/
disabled-aquifer path are also compared. Noise-based aquifer configuration and
fluid-pressure selection are implemented, with differential checks for seeded
decisions, sampling order, caches and failure recovery. Tag codecs, stacked resource
replacement and bounded dependency resolution are compared across all 300 bundled
block tags and synthetic missing-reference/cycle fixtures. Complete terrain
integration remains pending.

The [heightmap suite](heightmaps.md) records **1,671,668 matching output lines per
reference JAR** across all 35,723 block states and 100 chunk fixtures. Rust resolves
tag membership; block air/fluid facts are private, independently verified imports.
Actual vanilla heightmaps compare predicates, mutations, packed words, priming and
selected ordered block reads. Chunk-lifecycle integration remains pending.

The [noise-fill suite](terrain-filling.md) now compares **7,950,873 output lines per
reference JAR** across 114 fixtures and 200 fills. Final density, aquifers, resident
sections, world-generation heightmaps and fluid postprocessing are integrated for
this stage. The surface stage has a separate comparison; biome, feature and
structure stages remain pending.

The [structure-density sampler](structure-density.md) matches **1,360,095 output
lines per reference JAR**, including ordered rigid-piece/junction contributions
and scalar/bulk clipping. Live structure discovery and placement remain pending.

The [vertical-anchor suite](vertical-anchors.md) matches **181,632 output lines per
reference JAR**, including four anchor codecs, runtime overflow, nested exclusive
fields and generation bounds.

The [material-surface primitives](material-surfaces.md) match **122,988 output
lines per reference JAR** for seeded terracotta bands, band lookup and surface
noise queries. Subsequent suites cover rule evaluation and surface application.

The [numeric material-condition suite](material-conditions.md) matches **5,627,426
output lines per reference JAR** for eleven condition forms and their column/block
cache transitions, including codecs, all eight bundled condition definitions and
block/sequence/conditional/terracotta-band rule composition.
The chunk surface comparison below integrates these rules.

The [biome-temperature suite](biome-temperatures.md) matches **836,659 output lines
per reference JAR** for temperature modifiers, precipitation decisions and ordered
cache behavior, including thread isolation and climate fields from all 67 bundled
biome definitions. Full weather placement remains pending.

The [surface-extension suite](material-surfaces.md#biome-specific-column-extensions)
matches **233,100 output lines per reference JAR** for badlands pillars and
frozen-ocean icebergs, including ordered column reads/writes and both random modes.

The [bundled material-rule suite](material-rule-libraries.md) matches **678,174
output lines per reference JAR**, including 665,712 rule applications across all
42 bundled definitions and the pinned biome, noise and density registries.
The [chunk surface stage](material-surfaces.md#chunk-surface-stage) matches
**6,733,382 output lines per reference JAR** across 79 fixtures, 42 terrain fills
and 106 surface builds, followed by 158 carving-mask passes, including 21,757,952 block values, heightmaps, fluid
postprocessing and ordered biome queries. It tests all seven bundled terrain
settings through both stages with shared density contexts and vanilla fuzzy
biome lookup against 378 generated neighbor biome chunks (400,896 stored cells).
Preset builders and complete generated worlds remain pending.

The [biome-lookup suite](biome-lookup.md) matches **256,933 output lines per
reference JAR** for seed obfuscation, eight-corner fuzzy selection, direct quart
queries and source replacement. Seed hashes also match an independent SHA-256
implementation. [Chunk biome storage](biome-lookup.md#chunk-biome-storage) matches
**478,340 lines per JAR** across 160 fills, including section-atomic failure
behavior and clamped reads. [Chunk biome generation](biome-lookup.md#chunk-biome-generation)
matches **383,072 lines per JAR** across 322 fills, including both imported
multi-noise presets and fixed/checkerboard/End sources through all seven terrain
routers. Preset builders and complete world generation remain pending.

The [fixed/checkerboard/End source suite](biome-lookup.md#fixed-checkerboard-and-end-sources)
matches **1,306,096 lines per JAR**, including 1,147,984 biome queries and ordered
End erosion sampling. [Registry-bound source codecs](biome-lookup.md#registry-bound-source-codecs)
match **363,121 lines per JAR** across 40,481 inputs, including canonical encoding,
possible-biome sets and resolved queries. Chunk generation and surface integration
share the resolved-source API. Inline biome definitions and complete registry
lifecycle behavior remain pending.

The [carving-mask suite](carving.md) matches **45,742 lines per JAR** across
28 fixtures, including packed column ranges, coordinate wrapping and mutations
retained after failing traversal callbacks. Mask application is included in the
surface-stage suite. [Shared carving geometry](carving.md#shared-shape-primitives)
matches **106,699 lines per JAR**, including 690 ellipsoids and 5,536 reachability
queries. [Seeded cave/canyon paths](carving.md#seeded-cave-and-canyon-paths) match
**882,005** and **1,094,517 lines per JAR**, respectively. The combined stage suite
also covers 42 procedural carving passes, including neighboring biome selection
and ordinal carver seeds. Full-world integration remains pending.

The [feature-placement primitives](feature-placement.md) match **1,268,600 lines per
JAR** across seventeen modifier kinds, including callback failures and shared random
state. [Modifier-chain execution](feature-placement.md#modifier-chains) matches
**548,461 lines per JAR**, including feature callback failures and reused pending
buffers. [Initial block-writing features](feature-placement.md#initial-block-writing-features)
match **1,820,614 lines per JAR** for the void platform, End islands and End arrival
platform. Remaining feature algorithms and complete world integration remain pending.

The [scalar value-provider suite](value-providers.md) matches **429,920 lines per
JAR** across 504 definitions and 5,040 seeded fixtures, including exact random
stream consumption. [Anchored height providers](value-providers.md#anchored-height-providers)
match **507,660 lines per JAR**, including all four anchors and overflowing contexts.
Weighted lists include zero weights, nested choices and maximum totals. Gaussian
distributions and provider encoding remain pending.

The [block-state JSON codec suite](block-state-codecs.md) matches **221,261 output
lines per reference JAR**, covering every registered state and compact/full
encoding plus malformed-property fallback. Block gameplay remains pending.

## Work required for a playable rewrite

| Subsystem | Current Rust status | Required parity evidence |
| --- | --- | --- |
| Registries, block states, items and data components | Block-state IDs, property descriptors and defaults imported and checked; items/components absent | Registry lifecycle, remaining registries, data components and behavior |
| World/chunk storage, NBT and region files | Bounded NBT, four codecs, region export, saved and resident section palettes | Generated-save integration, entity/POI integration, upgrades, live saves and crash recovery |
| Terrain, biomes, structures and lighting | Seeded noise/density graphs, bundled registry and router loading, climate sampling/search and imported-table multi-noise biome resolution, aquifer decisions, noise-stage filling and surface-stage application implemented; complete terrain generation absent | Remaining terrain/biome codecs and world fields, seeded generated-chunk comparisons, biome/feature/structure placement, chunk lifecycle and light propagation |
| Tick scheduling, blocks, fluids and redstone | Scheduled queues, eligibility, callback mutations and saved tick data implemented; block/fluid/redstone behavior absent | Actual block/fluid state transitions, neighbor updates, world integration and end-to-end ordered traces |
| Entities, AI, movement and combat | Bounding-box and voxel collision geometry implemented; live entities absent | Context-dependent collision/physics and tick traces, spawning, pathfinding, damage, equipment and lifecycle |
| Survival, inventory, crafting and progression | Not implemented | Recipes, item transitions, hunger/health, effects, experience and advancement behavior |
| Dimensions, portals, weather and game rules | Not implemented | Cross-dimension state, transitions, time/weather and rule-dependent behavior |
| Multiplayer, protocol, authentication and commands | Not implemented | Packet/codec fixtures, vanilla interoperability, reconnects and command behavior |
| Rendering, resources, sound, UI and input | Not implemented | Resource compatibility, render/culling correctness, controls and playable interaction tests |
| Performance and platform support | Not measured for a Rust game | Repeated matched gameplay workloads, frame/tick distributions, memory and physical GPU tests |

The next simulation milestone is integrating the checked scheduler and
registry-backed sections with actual block behavior and neighbor updates. World
generation now has noise-stage filling, aquifers and surface-stage application;
biome assignment and remaining generation stages need work,
followed by complete seeded-world comparisons. A renderer needs
actual block models, neighboring sections and light data. Simplified geometry
alone cannot establish vanilla behavior. Performance tuning must preserve the
validated simulation and use measured workloads; changing render distance,
simulation distance or terrain detail invalidates an equal-workload speed claim.

No finite corpus proves every vanilla behavior. Future milestones need broader
version-specific differential suites and playable integration checks. The
project will report implemented/validated scope rather than label a partial
port as 100% parity.

The [pre-light chunk-stage comparison](terrain-filling.md#block-writes-across-generation-stages)
adds **8,090,090 matching lines per JAR** for stage-dependent block mutation and
heightmap selection, including insertion/removal of every pinned state. This does
not establish the complete chunk lifecycle or region integration.

The [resident feature-region comparison](feature-placement.md#resident-feature-regions)
adds **28,922,634 matching lines per JAR** for basic features, configured fill layers, replacements and scattered ore sharing
modifier chains and resident neighboring chunks, including live heightmap updates.
World-region side effects and complete feature generation remain pending.

The [inline feature-graph comparison](feature-placement.md#inline-feature-composition)
adds **1,146,057 matching lines per JAR** for nested sequences, overlays and random selectors,
including short-circuit results, shared randomness and partial mutations.

The [named feature-resource comparison](feature-placement.md#named-feature-resources)
adds **2,528,704 matching lines per JAR** for registry-aware feature/placed-feature
holders, ordered tags and subsequent nested execution. Complete registry loading,
reloads and remaining feature algorithms are still pending.

The [configured fill-layer comparison](feature-placement.md#configured-fill-layers)
adds **1,305,274 matching lines per JAR** for block-state binding and ordered
column placement, including decoding every pinned block state. Complete feature
generation and gameplay remain pending.

The [replacement-rule comparison](feature-placement.md#replacement-rules) adds
**1,890,716 matching lines per JAR** for all ten native predicate kinds, bound
state/tag identities and ordered random evaluation. Complete ore and structure
generation remain pending.

The [single-block replacement comparison](feature-placement.md#single-block-replacement)
adds **303,678 matching lines per JAR** for ordered rule-driven block mutations,
rejected writes, failures and subsequent random state.

The [scattered-ore comparison](feature-placement.md#scattered-ore) adds
**758,484 matching lines per JAR** for attempt offsets, ordered air-exposure checks,
rule evaluation and block mutations. Ordinary ore veins and complete biome
decoration remain pending.

The [bulk-section comparison](feature-placement.md#bulk-section-access) adds
**959,640 matching lines per JAR** for cached raw section access, every block state,
all-air reads and unchanged heightmaps. This supplies storage behavior needed by
ordinary ore generation; vein geometry and concurrent ownership remain pending.

The [biome feature-order comparison](feature-placement.md#biome-feature-ordering)
adds **411,751 matching lines per JAR** for identity-based ordering, cycles and
step index mappings, including every bundled biome's feature lists. This does
not yet execute complete chunk decoration.

The [world-generation random-adapter comparison](feature-placement.md#world-generation-random-streams)
adds **238,800 matching lines per JAR** for native bit sampling, counters, forks,
nested wrappers and decoration/feature seeding. Gaussian sampling and complete
chunk decoration remain pending.

The [biome-placement comparison](feature-placement.md#biome-placement-filtering)
adds **121,423 matching lines per JAR** for top-feature contexts, native generation
settings membership, query order, nested ordinary placers and partial failures.
Complete biome sampling integration and chunk decoration remain pending.

The [decoration scheduler comparison](feature-placement.md#decoration-feature-scheduling)
adds **337,398 matching lines per JAR** through the native chunk-decoration entry
point, with structures disabled and traced feature sinks. It covers resident
biome selection, global ordering, per-feature seeds and failure/progress state;
complete feature algorithms and chunk lifecycle remain pending.

The [resident decoration comparison](feature-placement.md#decoration-over-resident-chunks)
adds **36,173,951 matching lines per JAR** for native chunk decoration combining
biome storage/zoom, generation-stage guards, scheduling, implemented feature
graphs, block writes and live heightmaps. It compares complete final resident
sections; structures, lighting and the full generation lifecycle remain pending.

The [block-state provider comparison](feature-placement.md#block-state-providers)
adds **257,299,719 matching lines per JAR** for ten state-provider kinds, including
all eight bundled provider definitions. It includes every state under six
directions, property transforms, nested draw order and empty selections. Rule-based
selection includes twelve predicate kinds, ordered conditions, empty results and
query failures. Named holder references are resolved through immutable resource
snapshots. Additional predicates, complete registry loading and complete feature placement remain pending.

The [contained-fluid comparison](world-data.md#contained-fluid-states) adds
**243,331 matching lines per JAR** for fluid-state mapping, liquid-block conversion
and surface/camera height queries. It covers every pinned block state and query
failures; fluid spreading, entity interactions and rendering remain pending.

## Overlay feature composition

The Rust overlay feature executes every child placed feature in order and
accumulates success, sharing the placer and random stream across siblings.
Named holders and placed-feature tags use the same catalog resolution as other
compositions. Empty named tags produce false; empty direct lists fail decoding.

The expanded [inline graph comparison](../benchmarks/rust-feature-graph-2026-10-06/scoped-parity.json)
matches **1,146,057 output lines per reference JAR**, and the expanded
[named catalog comparison](../benchmarks/rust-feature-catalog-2026-10-06/scoped-parity.json)
matches **2,528,704 lines per reference JAR**. These include partial failures,
rejected writes, nested modifiers, repeated calls, random-stream tails and both
Java random modes. This verifies scoped composition behavior; complete chunk
generation and gameplay remain pending. Workspace tests and all-target Clippy
also pass for this implementation.

## Block-pile placement

Rust block piles preserve radius and sparse-candidate random draws, air checks,
dirt-path acceptance, state-aware FULL upper support, provider fallback reads,
and update flags 260. Previously read support states are evaluated without an
extra world read. The [native comparison](../benchmarks/rust-block-pile-2026-10-06/scoped-parity.json)
matches **11,995,254 lines per reference JAR**, covering every candidate and
supporting registry state, partial failures, rejected writes, repeated placement,
and overlay/modifier composition. Workspace tests and all-target Clippy pass.

Ordinary piles visit at most 98 cells. Rust preserves wrapped native box traversal
but limits pathological iteration to 2^20 cells; negative-count overflow fixtures
compare failure-terminated prefixes. Dynamic block entities are absent in this
feature fixture, and resident-world block updates remain pending.

## Disk replacement and postprocessing

Disk features now decode bounded radii and half-heights, traverse circular
columns in native order, apply optional providers, and mark up to two non-air
blocks above each replacement run. Failed target predicates end a run; empty
optional providers preserve it. Postprocessing requests resolve the containing
resident chunk and retain duplicate marks and prior effects after a failure.

The [disk comparison](../benchmarks/rust-disk-2026-10-06/scoped-parity.json)
matches **2,816,586 output lines per reference JAR** across **76,409 fixtures**.
Every candidate state and every possible above-state is covered, alongside
random providers, rejected writes, partial failures, wrapped coordinates and
composed placement. Chunk-resolution and native ProtoChunk mark calls are
traced; later chunk postprocessing execution is outside this comparison.
Workspace tests, all-target Clippy and all 149 Python tests pass.

## Block-blob placement

Block blobs descend until their support predicate accepts the block below, then
write three overlapping randomized shapes with flags 3. Radius calculations
retain native float precision while position distances use double precision;
even the last shape consumes its three origin-offset draws.

The [block-blob comparison](../benchmarks/rust-block-blob-2026-10-06/scoped-parity.json)
matches **2,387,914 lines per reference JAR** over **77,686 fixtures**, including
every candidate/supporting state, repeated calls, early returns, failed queries,
rejected writes, partial failures and overlay/modifier composition. Workspace
tests and all-target Clippy pass. Rust bounds pathological traversal work;
resident-world update callbacks and full gameplay remain pending.

## Fluid-state JSON codecs

Fluid states now decode and encode native object-form identifiers and properties.
Missing properties retain the pinned defaults (`falling=true`, flowing level 1);
invalid property values are ignored, while unknown fluid identifiers fail.
Integer levels share the Java-compatible decimal parser with block properties.

The expanded [fluid comparison](../benchmarks/rust-fluid-2026-10-06/scoped-parity.json)
matches **243,331 lines per reference JAR**, including all 37 fluid states,
2,432 codec fixtures, encoding, legacy block conversion, and the existing
contained-fluid/height tests. Workspace tests and all-target Clippy pass.
Fluid spreading, tick execution and entity interaction remain pending.

## Spring features and fluid tick requests

Spring features preserve above/below/origin gating, the separate west/east/north/
south/below rock and air-count passes, fluid-to-block conversion, writes with
flags 2, and immediate fluid tick requests after accepted or rejected writes.
Fluid tick execution is still separate simulation work.

The [spring comparison](../benchmarks/rust-spring-2026-10-06/scoped-parity.json)
matches **3,649,087 lines per reference JAR** across **172,141 fixtures**. It
covers every state in the gating and neighbor roles, all five-neighbor air/rock/
other combinations and count pairs, all 37 fluid states, malformed fields, tags,
wrapped positions, partial failures and nested composition. Workspace tests and
all-target Clippy pass. Full fluid spreading and world updates remain pending.

## Vine feature attachment

Vine features require an air state at the origin and choose the first acceptable
attachment in Up/North/South/West/East order. They evaluate actual support and
collision shapes and return success after the write request, including when the
world rejects that write. Feature and world random streams are unchanged.

The [vine comparison](../benchmarks/rust-vines-2026-10-06/scoped-parity.json)
matches **4,315,682 lines per reference JAR** across **224,743 fixtures**, covering
every origin state and every state on each candidate face, support combinations,
wrapped positions, partial failures and composed placement. Workspace tests and
all-target Clippy pass. Vine growth, block updates and complete gameplay remain
pending; this fixture uses absent dynamic block entities.

## Full-face occlusion

Block property bindings now cache six full-face occlusion results per state,
sharing calculations for identical imported shapes. These results use native
occlusion rules independently of collision or support geometry. Unknown state
IDs produce an error.

The expanded [block-property comparison](../benchmarks/rust-block-properties-2026-10-06/scoped-parity.json)
matches **197,180 lines per reference JAR**, including all 35,723 states and
18,096 synthetic shape/flag combinations. Workspace tests and all-target Clippy
pass. Lighting and renderer integration remain separate work.

## Underwater magma and probability decoding

Underwater magma now performs native water-column scans, including the upper
scan before the lower scan and repeated boundary reads. Each cube position draws
a probability value before testing block identity and the five required full
occlusion faces. Rejected writes still count as successful placements.

The [underwater magma comparison](../benchmarks/rust-underwater-magma-2026-10-06/scoped-parity.json)
matches **6,523,072 lines per reference JAR** over **253,252 fixtures**. Both RNG
modes cover the complete radius-64 cube, consuming **2,146,689 draws** with zero
placement probability. Every state in the origin/floor/neighbor roles, scan
limits, partial failures, wrapped radius-zero columns and composition are tested.

The native comparison exposed signed-zero codec behavior: bounded probabilities
reject `-0.0`, including negative values that round to float zero. Magma and
pillar decoding now preserve this. The expanded pillar gate passes **921,111
lines per JAR** over **41,746 fixtures**. Workspace tests and all-target Clippy
pass. World updates and magma/fluid gameplay remain pending.

## Blue ice generation

Blue ice features now preserve world sea-level gating, water and packed-ice
seeding checks, all 200 spreading attempts, signed vertical radius adjustment,
random draw order and six-direction adjacency. World sea level can differ from
the generation context's sea level. Rejected writes preserve native success and
subsequent attempts observe the actual mutable world.

The [blue-ice comparison](../benchmarks/rust-blue-ice-2026-10-06/scoped-parity.json)
matches **8,954,666 lines per reference JAR** across **79,944 fixtures**, covering
every origin/candidate state through failure-terminated prefixes, full spreading
runs, all packed-ice neighbor masks, extreme sea levels, wrapped positions,
partial failures and composition. Workspace tests, all-target Clippy and all
149 Python tests pass. Ice gameplay and complete world generation remain pending.

## Spike geometry and roots

Spike features now implement descending air search, support predicates, randomized
height and radius, rare elevated spikes, float taper calculations and independent
upper/lower replacement tests. Roots retain the native fixed Y=50 boundary,
corner-specific random lengths, gaps, and exact-state replacement exception.

The [spike comparison](../benchmarks/rust-spike-2026-10-06/scoped-parity.json)
matches **8,859,761 lines per reference JAR** over **113,127 fixtures**. Every
output/origin/candidate state is covered by failure-terminated prefixes; complete
runs include 1,024 seeds per RNG, replacement and support predicates, repeated
mutations, rejected writes, partial failures and composition. Workspace tests
and all-target Clippy pass. Complete world generation and block updates remain
pending. Pathological descent/root work is bounded to 2^20 visits.

## End exit podium construction

End podium features reproduce active/inactive construction, the native distance
cutoffs and box traversal, central bedrock pillar and ordered wall-torch facing.
Active replacement reads block identity, requests destruction with drops and no
responsible entity, then writes the replacement even if destruction was rejected.
Matching blocks skip both requests.

The [End podium comparison](../benchmarks/rust-end-podium-2026-10-06/scoped-parity.json)
matches **3,309,219 lines per reference JAR** over **38,207 fixtures**. All states
reach active replacement prefixes; full runs cover representative blocks, repeated
construction, rejected destruction/writes, partial failures, composition and
nonoverflow extreme coordinates. Workspace tests, all-target Clippy and all
149 Python tests pass. The fixture controls destruction success and air
replacement; loot, block entities, updates, portal travel and dragon gameplay
remain pending. Pathological overflow-box work is bounded to 2^20 visits.

## Replacement blobs

The `netherrack_replace_blobs` feature now clamps the search origin to world
height, descends to a matching block, samples three independent radii and visits
clipped Manhattan shells in native order. Target matching uses block identity
regardless of state properties; rejected writes still produce success.

The [replacement-blob comparison](../benchmarks/rust-replace-blobs-2026-10-06/scoped-parity.json)
matches **5,538,034 lines per reference JAR** over **109,003 fixtures**, including
every target/output state, all candidate states through failure-terminated
prefixes, every constant radius, six provider families, both RNGs, height
boundaries, wrapped X/Z coordinates, partial failures and composition. Workspace
tests and all-target Clippy pass. World updates and full generation remain
pending; pathological descent is bounded to 2^20 visits.

## Bamboo and podzol patches

Bamboo features preserve the initial air gate, real survival checks and native
attempt-success behavior when survival fails. Podzol patches use ordered surface
height requests and the native replacement tag. Stalks preserve random draws and
leaf states, including the first cap overwriting the stopping position when at
least three stalk positions were attempted.

The [bamboo comparison](../benchmarks/rust-bamboo-2026-10-06/scoped-parity.json)
matches **3,485,027 lines per reference JAR** over **144,452 fixtures**. Every
origin/support/podzol state has a failure-terminated prefix; every obstacle state
has a complete no-podzol run. Tests include both RNGs, signed-zero decoding,
obstruction heights, rejected writes, repeated mutations, wrapped coordinates
and composition. Workspace tests and all-target Clippy pass. Fixture surface
heights are controlled; dynamic height updates, bamboo growth and full gameplay
remain pending. Pathological wrapped patch loops have a 2^20 work limit.

## Nether delta patches

Delta features now preserve the double-precision rim chance, conditional rim
offset draws, independent patch radius draws and Manhattan traversal. Placement
checks block identity and ordered air/nonair neighbors, writes the rim, then
checks shifted contents against the mutated world. A zero rim component still
shifts contents even when it disables the rim write.

The [delta comparison](../benchmarks/rust-delta-2026-10-06/scoped-parity.json)
matches **13,418,994 lines per reference JAR** across **324,269 fixtures**,
covering every origin/content/rim state and all six neighbor roles, every air
mask, radius limits, six provider families, both RNGs, repeated mutations,
rejected writes, partial failures, wrapped positions and composition. Workspace
tests, all-target Clippy and all 149 Python tests pass. The fixture supplies a
flat ground/air boundary with mutable overrides; fluid behavior, world updates
and complete generation remain pending.

## Biome freezing and snow eligibility

Registry-bound weather predicates now use the native biome temperature cache,
precipitation selection, build-height and block-light gates. Freezing distinguishes
source water from flowing water, requires a liquid block and optionally checks
water neighbors in west/east/north/south order. Snow accepts air or existing snow
and evaluates the default snow state's actual survival rules.

The [biome-weather comparison](../benchmarks/rust-biome-weather-2026-10-06/scoped-parity.json)
matches **4,071,594 lines per reference JAR** across **326,057 fixtures**,
covering all states in block/fluid/snow/support/neighbor roles, neighbor masks,
float thresholds and nonfinite temperatures, both temperature modifiers, light
and height boundaries, precipitation flags, repeated queries and partial failures.
Workspace tests and all-target Clippy pass. Native fixture temperature caches
are removed after each independent case to avoid retaining hundreds of thousands
of biomes; repeated queries within a case keep their cache. Dynamic block entities
are absent. Actual weather ticks, lighting propagation and world updates remain
pending.

## Frozen top layers and snowy ground

The `freeze_top_layer` feature now visits its 16×16 footprint in native column
order, queries motion-blocking height and retains the selected biome for both
weather checks. Ice is attempted before snow eligibility. Snow attempts are
followed by snowy ground-property updates even if the snow write was rejected.

The [snow/freezing feature comparison](../benchmarks/rust-snow-freeze-2026-10-06/scoped-parity.json)
matches **7,764,082 lines per reference JAR** over **108,339 fixtures**. Every
ground/candidate/fluid state has a failure-terminated prefix; full runs cover
climate thresholds and NaN, both modifiers, precipitation, light gates, repeated
mutations, rejected writes, partial failures, extreme heights/sea levels, wrapped
coordinates and composition. Workspace tests and all-target Clippy pass. The
fixture uses controlled heights and one biome; lighting propagation, dynamic
biome selection, world updates and full generation remain pending.

## Generation-stage tick containers

Proto-chunk tick containers now keep the first type/position request in insertion
order. Newly scheduled ticks retain priority with zero delay; loaded entries
retain their saved delay. Generation tick access routes requests to the selected
container and reports no currently executing ticks or aggregate count. Snapshots
can be promoted into the existing level tick queue.

The [generation-tick comparison](../benchmarks/rust-generation-ticks-2026-10-06/scoped-parity.json)
matches **101,784 lines per reference JAR**, covering duplicates, saved entries,
snapshot independence, mixed proto/level routing, lookup failures, promotion,
priority boundaries and integer/time extremes. Workspace tests, all-target Clippy
and all 149 Python tests pass. Rust exposes packed entries as an immutable
borrowed slice; mutating Java's returned list is outside this API. Feature-world
scheduling integration and block/fluid tick execution remain pending.

## Resident generation tick queues

Resident terrain chunks now own separate block and fluid proto-tick queues.
Feature-world scheduling normalizes block states to block identity, routes to the
chunk containing the original position and advances the region's tick order even
for duplicate requests or missing chunks. Time and order use Java wrapping rules.

The [resident queue comparison](../benchmarks/rust-feature-region-ticks-2026-10-06/scoped-parity.json)
matches **28,985,883 lines per reference JAR** across **192 fixtures**, with nine
resident chunks each, **2,688 requested feature placements** and **14,784 direct
tick requests**. Both references agree on final sections, heightmaps and saved
ticks. Coverage includes spring scheduling, duplicate block identities, separate
fluid identities, missing chunks, out-of-height positions and time/order overflow.
Workspace tests and all-target Clippy pass. This connects generation-stage queues;
block/fluid tick execution, complete chunk persistence and full world-generation
effects remain pending.

## Resident block and fluid queries

Feature regions can bind an immutable snapshot of resolved block/fluid tags and
contained fluid states. The snapshot shares compiled tables with configured
features and rejects bindings from a different block registry. Feature access
forwards fluid, water-tag and block-tag queries against current resident storage.
Missing tags have no members; missing chunks remain query errors.

The [resident query comparison](../benchmarks/rust-feature-region-queries-2026-10-06/scoped-parity.json)
matches **29,312,766 lines per reference JAR**, retaining the 192 multi-chunk
feature/tick fixtures and adding insertion/removal probes for **all 35,723 block
states**. Each probe checks fluid identity, amount, falling/source/full flags,
water membership and all 300 bundled block tags plus an absent tag. Negative
coordinates, chunk edges, outside-height reads and missing chunks are included.
Workspace tests and all-target Clippy pass; a separate regression covers foreign
registries, binding lifetime and a custom water tag. These queries do not execute
fluid spreading, block behavior or lighting.

## End gateway structures and exit configuration

The `end_gateway` feature now places its air, bedrock and gateway blocks in native
box order. A configured exit triggers a block-entity lookup even when the gateway
write was rejected. Missing or unrelated entities are no-ops; a gateway receives
its exit/exact fields before its dirty notification.

The [gateway comparison](../benchmarks/rust-end-gateway-2026-10-06/scoped-parity.json)
matches **754,877 lines per reference JAR** across **5,820 fixtures**, including
every world-call failure boundary, rejected writes, repeated updates, both random
streams, placement composition, codec edge cases and bounded prefixes of wrapped
coordinate boxes. The reference invokes the actual gateway entity setter and
compares its fields after partial failures. Workspace tests, all-target Clippy and
all 149 Python tests pass. The Python checks also caught and repaired a shared
fixture dependency: decoration retains its six-feature corpus independently of
region spring/tick/query probes; regenerated inputs are byte-identical to their
validated corpora. Gateway entity creation/removal, dirty propagation, exit search
and teleportation remain pending.

## Bonus chest placement and loot assignment

Bonus chest generation now independently shuffles the chunk's X and Z columns,
searches motion-blocking-no-leaves heights and checks air or empty collision.
After a chest write it looks up a randomizable container, consuming a loot seed
only when one exists, then checks torch survival in native horizontal order.
Rejected writes retain the subsequent entity and torch operations.

The [bonus-chest comparison](../benchmarks/rust-bonus-chest-2026-10-06/scoped-parity.json)
matches **1,505,542 lines per reference JAR** across **42,396 fixtures**. All
35,723 states participate in collision prefixes; complete searches cover both
random streams, missing/unrelated/chest entities, sole viable columns, exhausted
searches, repeated mutation, rejected writes, partial failures, extreme positions
and placement composition. Actual native chest loot-table and seed fields are
compared. Workspace tests and all-target Clippy pass. Heights and entity existence
are controlled by the fixture; inventory contents, loot generation, automatic
entity lifecycle and complete spawn generation remain pending.

## Recursive chorus generation

Chorus generation now checks its air origin and tagged support, builds and
refreshes connection states, branches recursively within the native depth and
distance bounds, and places age-five terminal flowers. Clearance checks exclude
the parent branch, and rejected writes retain native subsequent reads and draws.

The [chorus comparison](../benchmarks/rust-chorus-plant-2026-10-06/scoped-parity.json)
matches **8,083,367 lines per reference JAR** across **254,296 fixtures**. Every
state participates in origin/support gates and five neighbor-connection prefixes.
Full growth covers 1,024 seeds per random generator, early failure boundaries,
rejected writes, repeated mutation, random obstacles, wrapped coordinates and
feature composition. Workspace tests and all-target Clippy pass. Neighbor-update
execution, random tick growth, drops, rendering and complete End generation remain
pending.

## Small pointed formations

The `speleothem` feature now chooses its growth direction from ceiling/floor
blocks, spreads a base patch, selects one- or two-block height, rechecks its base
and writes waterlogged tip/frustum states. Configured base and pointed states
select block identities; placement uses their defaults. Unsupported pointed-block
properties fail during placement after the preceding patch operations.

The [speleothem comparison](../benchmarks/rust-speleothem-2026-10-06/scoped-parity.json)
matches **2,752,289 lines per reference JAR** across **145,473 fixtures**, including
all states in support, taller-growth and contained-fluid roles; optional float
defaults and boundaries; both random streams; rejected writes; partial failures;
repeated mutation; wrapped positions; and composition. Workspace tests and
all-target Clippy pass. Neighbor updates, falling behavior, fluid simulation,
larger clusters and complete cave generation remain pending.

## Generation-time destruction on resident chunks

Feature access now routes destruction through its supplied write callback using
air and update flags `3`. Air is an immediate false result. This follows
`WorldGenRegion.destroyBlock`, whose generation-time path does not evaluate drops;
live-level destruction remains separate work.

The [resident destruction comparison](../benchmarks/rust-feature-region-destroy-2026-10-06/scoped-parity.json)
matches **29,502,523 lines per reference JAR** across **192 fixtures**, adding
active/inactive End podiums to the resident feature corpus. The reference invokes
the actual WorldGenRegion method through a subclass that transports reads/writes
to the existing chunk fixture. Final sections, heightmaps, ticks, tag/fluid probes,
rejected writes and partial failures still agree. Workspace tests, all-target
Clippy and all 149 Python tests pass. Write-zone policy, POIs, block entities,
postprocessing execution, lighting and live-world effects remain pending.

## Generation write radii

Feature access can now apply the generation step's horizontal write radius before
calling the mutation policy. Ordinary writes and destruction share the guard;
queries and tick scheduling retain their independent routing. The predicate uses
Java wrapping subtraction and absolute value, including integer-minimum cases.

The [write-zone comparison](../benchmarks/rust-feature-region-write-zone-2026-10-06/scoped-parity.json)
matches **29,231,123 lines per reference JAR**. It adds **20,125 direct zone cases**
for both chunk and block coordinates, then exercises unrestricted, zero-radius,
shifted-center and negative-radius policies across the 192 resident fixtures.
Vanilla's actual public/private write-zone methods provide the reference. Workspace
tests, all-target Clippy and both focused fixture tests pass. The owning world
still supplies generation-step metadata and upgrading-world vertical restrictions;
full WorldGenRegion effects remain pending.

## Proto-chunk block-entity storage

Rust now stores pending NBT and caller-owned active block-entity payloads by full
block position. Active insertion removes pending data, later pending insertion
cannot overwrite an active entry, and removal clears both maps. Saving uses the
active serializer when present and otherwise returns a pending snapshot. Changing
pending coordinate fields retains the existing map key. Coordinate resolution
matches NumericTag integer narrowing and floating-point flooring before saturating
conversion, then relocates mismatched X/Z into the owning chunk.

The [block-entity storage comparison](../benchmarks/rust-block-entity-storage-2026-10-06/scoped-parity.json)
matches **78,251 lines per reference JAR** across **28,511 commands**, using actual
native ProtoChunks and chest entities. Cases cover replacement, removal, position
unions, save selection, missing/nonnumeric coordinates, infinities/NaNs, integer
extremes and positions that alias when packed. Four focused Rust tests and
all-target Clippy pass. Collection order is normalized; Rust exposes borrowed or
owned data instead of unrestricted Java aliases. Entity construction from NBT,
complete serializers, active entity behavior and resident chunk lifecycle
integration remain pending.

## Upgrading-world write heights

Feature access can now enforce the upgrading center chunk's generation height
before invoking its write callback. Below-zero proto-chunk upgrades use Y = -64
through -1, inclusive. Ordinary generation has no vertical policy gate; storage
bounds remain separate. Reads and tick scheduling keep their existing behavior,
and generation-time destruction shares the write guard.

The [upgrade-height comparison](../benchmarks/rust-feature-region-upgrade-height-2026-10-06/scoped-parity.json)
matches **29,247,433 lines per reference JAR**, including **10,098 height cases**,
20,125 horizontal zone cases and all 192 resident fixtures. The reference calls
actual `WorldGenRegion.ensureCanWrite` with a native ProtoChunk before and after
installing below-zero upgrade data. Direct write probes cover boundary and extreme
heights, ordinary/upgrading modes and horizontal rejection. Five focused Rust
region tests, all-target Clippy and both Python fixture tests passed for that
checkpoint. Subsequent work below adds resident upgrade metadata, cleanup,
bedrock transformations and live center-chunk write policy. Automatic upgrade
activation, biome retention and the complete generation lifecycle remain pending.

## Chunk-status identities and metadata

Rust now represents all ten pinned chunk statuses, their ordering and parents,
proto/level chunk classification, structure-distance constant, and selected
heightmaps. Existing pre-light chunks use these definitions for their names and
heightmap groups. Direct name lookup retains vanilla's `empty` fallback; NBT
codec lookup rejects unknown names and writes canonical namespaced identifiers.

The [chunk-status comparison](../benchmarks/rust-chunk-status-2026-10-06/scoped-parity.json)
matches **4,232 lines per reference JAR** across all 100 ordered status pairs and
**2,065 codec inputs**. Inputs include qualified/unqualified names, empty
namespaces, obsolete and malformed names, random strings and nonstring NBT.
World-crate tests and all-target Clippy pass. Codec error text is normalized.
Status identities do not execute generation tasks: scheduling, lighting,
post-light lifecycle transitions and the complete gameplay runtime remain pending.

## Below-zero upgrade data and resident transformations

Resident chunks now hold optional below-zero upgrade data. Installation alone
keeps the data even if the current stage already exceeds its target; assigning a
persisted stage at or beyond that target clears it. Upgrading chunks expose the
-64 through -1 generation interval independently of their storage bounds.

The Rust upgrade codec validates nonempty target statuses, accepts vanilla's
lenient optional bedrock-hole mask, normalizes trailing zero words and omits an
empty mask when saving. Hole queries wrap local X/Z coordinates. Resident
transformations replace bedrock in the old Y = 0 through 4 band with default
deepslate and clear marked columns through the chunk's current generation height.

The [upgrade-data comparison](../benchmarks/rust-retrogen-data-2026-10-06/scoped-parity.json)
matches **20,626 lines per reference JAR** across **5,114 codec inputs**, including
canonical output, all 256 column bits, out-of-footprint bits, signed numeric
streams, malformed optional fields and random masks. Codec error text is normalized.
The [resident upgrade comparison](../benchmarks/rust-retrogen-chunk-2026-10-06/scoped-parity.json)
matches **3,028,057 lines per reference JAR** across **138 fixtures**. It checks all
nine nonempty targets against the six pre-light stages, repeated/backward stage
assignments, complete stored sections and heightmap words after replacement and
masking, partial vertical overlap, empty sections and changed generation bounds.
World-crate tests and all-target Clippy pass. Save-dirty notifications, post-light
transitions, biome retention, structure restrictions, automatic generation
scheduling and full saved-world/gameplay integration remain pending.

## Live upgrade permissions and retained biome status

Feature access can bind a resident center chunk and read its upgrade state before
every write. Successful callbacks that complete, remove or reinstall an upgrade
affect the next write through the same access object. The destination chunk's
upgrade state does not select this region-wide height gate. Explicit fixed-height
policies remain available for callers that need a supplied interval.

Resident chunks also expose vanilla's highest generated status: the later of
their current persisted stage and retained upgrade target. Region biome reads use
this status, allowing access to old biomes while below-zero regeneration is still
at an earlier stage. Removing upgrade metadata restores the persisted-stage gate.

The [live region comparison](../benchmarks/rust-live-upgrade-region-2026-10-06/scoped-parity.json)
matches **29,248,297 lines per reference JAR**, extending the 192 resident fixtures
with **54 lifecycle cases**. It compares native write permission, highest status
and actual ProtoChunk biome-read availability across all six pre-light stages and
nine nonempty upgrade targets. The [retained-status comparison](../benchmarks/rust-retained-upgrade-status-2026-10-06/scoped-parity.json)
also matches **3,028,057 lines per JAR** across 138 upgrade fixtures. Seven focused
region tests, all-target Clippy and the region corpus test pass. These checks do
not yet cover the biome resolver's cave-biome retention, structure restrictions,
automatic scheduling or complete saved-world integration.

## Retained biomes during below-zero generation

The [biome-retention implementation](biome-lookup.md#below-zero-biome-retention)
now preserves the pinned three cave-biome candidates and otherwise reads the old
column at quart Y = 0. The resolver captures upgrade activation at construction;
candidate failures precede fallback reads. Section fills preserve native commit
ordering and partial-failure behavior while allowing access to old biome storage.

The [retention/fill comparison](../benchmarks/rust-retrogen-biome-fill-2026-10-06/scoped-parity.json)
matches **593,152 lines per JAR** across 740 fills and 2,800 direct queries. The
[generation integration](../benchmarks/rust-retrogen-biome-generation-2026-10-06/scoped-parity.json)
matches **834,080 lines per JAR** across 396 fixtures and 610 fills, including 144
upgrading chunks alongside the ordinary-generation cases. It executes actual
native `ChunkGenerator.doCreateBiomes`, including retained-status old biome reads,
native below-zero generation bounds, repeated fills and all biome-source kinds.
This closes the resolver gap described in the preceding checkpoint. Nonempty
blending, structure restrictions, automatic generation scheduling and complete
saved-world/gameplay integration remain pending.
