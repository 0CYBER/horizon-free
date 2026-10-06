# Horizon Rust rewrite status

The behavior target is the repository's pinned **Minecraft Java Edition 26.3**.
One-to-one vanilla gameplay parity is the acceptance criterion. It has **not**
been reached: Horizon's Rust code does not yet run a client, server, or simulation.
It can read and export region files and access saved and resident block sections.
Scheduled block/fluid queues can now collect and dispatch callbacks with
vanilla-checked ordering; the actual block/fluid behavior is still absent.
Double-precision geometry and voxel-grid queries are implemented, with a registry
adapter for vanilla's no-context cached collision branch.
Base noise samplers, octave stacks, normal-noise definitions and bulk density
sampling are implemented. Compiled density samplers now evaluate constants, gradients,
shifted noise, all eleven unary operators, rounding and selected arithmetic operations. An initial density graph compiler
integrates range analysis, specialization, uniform-axis slicing, shared caches,
nested splines and seeded noise;
complete graph coverage, serialized graphs and generated terrain remain absent.
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

Requires Rust 1.85 or newer, Python 3.10 or newer, and JDK 25. From the checkout:

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
adds **737,779 matching lines per JAR** for nested sequences and random selectors,
including short-circuit results, shared randomness and partial mutations.

The [named feature-resource comparison](feature-placement.md#named-feature-resources)
adds **1,734,390 matching lines per JAR** for registry-aware feature/placed-feature
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
adds **105,340,143 matching lines per JAR** for ten state-provider kinds, including
all eight bundled provider definitions. It includes every state under six
directions, property transforms, nested draw order and empty selections. Rule-based
selection includes twelve predicate kinds, ordered conditions, empty results and
query failures. Named holder references are resolved through immutable resource
snapshots. Additional predicates, complete registry loading and complete feature placement remain pending.

The [contained-fluid comparison](world-data.md#contained-fluid-states) adds
**239,190 matching lines per JAR** for fluid-state mapping, liquid-block conversion
and surface/camera height queries. It covers every pinned block state and query
failures; fluid spreading, entity interactions and rendering remain pending.
