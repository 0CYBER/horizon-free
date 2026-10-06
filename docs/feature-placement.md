# Feature placement

Horizon implements eighteen Java Edition **26.3** placement modifiers: count,
square spread, three-axis offset, height range, rarity filtering, random-chance
filtering, fixed positions, random selection among modifiers, heightmap positions, surface-relative
thresholds, water-depth filtering, both noise-based count forms, cuboid placement,
count-on-every-layer scans, predicate filtering, vertical environment scans and biome filtering.

Placement preserves callback order, duplicate positions, wrapping coordinates,
provider sampling and random draw consumption. Fixed positions are filtered by
the origin's chunk X/Z. Random-choice nesting is bounded to depth 256 and 65,536
modifier nodes; lists are bounded to 8,192 entries. Consumer failures retain
previous emissions and consumed draws. Probability-range decoding distinguishes
negative zero from positive zero, matching the native codec.

Run `python3 tools/test_placement_parity.py`. The
[local report](../benchmarks/rust-placement-2026-10-06/scoped-parity.json) records
**1,268,600 matching lines per reference JAR**, across **520 definitions** and
**21,840 seeded fixtures**, with **70,080 requested modification calls** and **17,280 direct count queries**. Actual
`PlacementModifier.CODEC` and modifier callbacks supply the reference results.
The corpus tests both random modes, relative-height overflow, nested choices,
malformed fields, repeated calls, consumers that draw randomness and failures
before or after earlier emissions.

This suite uses an actual placement context backed by a world proxy with traced synthetic heightmaps.
Additional block predicates, full feature registries, registry integration,
Gaussian providers, encoding and complete generated worlds remain pending.


## Modifier chains

`FeaturePlacer` collects a modifier's outputs before processing its descendants,
preserving sibling order through a reusable pending stack. Actual feature callbacks
share the modifier random stream. Boolean results accumulate without skipping later
placements. An empty chain calls the feature directly. Errors retain pending and
modified-position buffers, matching vanilla when the same placer is used again.

Run `python3 tools/test_feature_placement_parity.py`. The
[chain report](../benchmarks/rust-feature-placement-2026-10-06/scoped-parity.json)
records **548,461 matching lines per JAR**. Actual native `FeaturePlacer` executes
native modifiers and a synthetic feature sink, comparing emitted positions,
feature results, consumed randomness and repeated calls after failures. This tests
the execution machinery; the separate feature suite below connects it to initial block-writing algorithms.

Rust bounds chains to 1,024 modifiers and per-call work/pending buffers to 2^20.
Unit tests reject oversized chains before feature calls or random draws and stop
multiplicative count expansion after bounded partial progress.


## Heightmap queries

`PlacementContext` supplies generation bounds, the world minimum and heightmap
queries. These are separate: the world minimum can differ from the generator's
minimum. Heightmap placement emits only above the world minimum. Surface-relative
thresholds use 64-bit sums; water-depth subtraction retains Java integer wrapping.
Queries and emitted positions retain native order, and query errors stop the
current operation without drawing randomness or emitting a replacement position.

Both suites include ordinary/extreme synthetic heights, all six map types,
distinct world/generator minima, default/reversed threshold ranges and injected
query failures. This validates how modifiers consume heightmaps; connecting
feature generation to complete world regions remains pending.


## Noise-based counts

Noise-based and noise-threshold counts share vanilla's fixed biome-information
simplex noise. They preserve the float noise result before double arithmetic,
ceiling/narrowing behavior and strict threshold comparison. They consume no caller
random draws. `repetitions` exposes counts without expanding position lists.

Primitive comparisons include extreme ratios, factors, offsets and thresholds,
including count-only queries that can return enormous counts safely. Chain tests
exercise ordinary noise counts together with other modifiers and feature callbacks.
Emitting repeating modifiers remains bounded to 4,096 repetitions in Rust.


Cuboid placement samples Y size, then independent X and Z sizes, before emitting
positions in X/Y/Z order. Its extents are inclusive. Edge and interior controls
are independent, and coordinates wrap. The suites cover all four control
combinations, provider variation, maximum decoded extents, consumer failures and
cuboids embedded in modifier chains.


## Layered placement

Count-on-every-layer placement samples its count provider at every loop-condition
check, including the final check. Each attempt draws X/Z offsets, queries the
motion-blocking heightmap and scans down for the requested surface layer. Air
states and exact water/lava blocks count as empty. Bedrock is excluded as ground;
other blocks, including bubble columns, are potential ground. Emissions and count
sampling interleave, sharing the caller's random stream.

The suites compare traced synthetic strata containing these block kinds, random
count providers, repeated passes and injected height/block/consumer failures.
`LayerBlock` is a query classification supplied by the world; complete block
behavior and region-backed feature generation remain pending. A shared 65,536-step
budget bounds scans, including when world queries supply extreme heights.


## Initial block-writing features

`BasicFeature` implements the void start platform, End islands and the End
arrival platform. The world supplies block reads and mutations; required block
states bind through its registry. The algorithms preserve write order, update
flags, wrapped coordinates, ignored false mutation results and partial effects
when a world operation fails. End-island radius sampling retains native draw order.

Run `python3 tools/test_basic_feature_parity.py`. The
[feature report](../benchmarks/rust-basic-feature-2026-10-06/scoped-parity.json)
records **1,820,614 matching lines per JAR**. Actual feature codecs and native place
methods operate on a mutable world proxy. Both random modes, multiple seeds,
nearby/far chunks, extreme coordinates, rejected writes, thrown failures and
repeated placements are compared, including complete accumulated stored states.
Native `setBlockAndUpdate` executes its actual default method, exposing its flags.
The suite also composes these features with modifier chains, sharing the mutable
world and random stream and reusing placer buffers after failures.
`place_with_context` gives feature callbacks the same world used by modifiers.

This covers feature algorithms against a supplied mutation interface. Actual chunk
mutation, neighbor updates, lighting, entities/drops, complete feature registries
and generated worlds remain pending. The End-platform generation path does not
request the separate block-destruction/drop behavior used by portal recreation.


## Predicate filters and environment scans

Logical predicates (`true`, `not`, `all_of`, `any_of`) preserve ordered
short-circuiting, including empty-list identities. Inside-world-bounds predicates
apply wrapping offsets before checking the world's inclusive height bounds.
Their offset codec retains Java's wrapping absolute-value behavior for the
minimum signed integer. These five predicate kinds also work inside filters and
vertical scans; state-dependent predicates remain pending.

Environment scans check the initial allowed condition before the target, then
move one block at a time. They stop immediately outside build height. A failed
allowed condition after a move still permits the final target test at that
position. The codec limits scans to 1–32 steps; shared work and nesting budgets
also bound directly constructed predicates and scans.

Both placement suites compare actual native codecs and predicate execution,
traced world-bound queries, short-circuiting, wrapped offsets, invalid directions,
step limits and query failures. Unit tests exercise oversized trees and scans
that would otherwise traverse billions of positions.


## Resident feature regions

`TerrainRegion` routes block reads/writes by signed chunk coordinates and lazily
primes missing heightmaps. It requires consistent vertical bounds and registry
identity across resident chunks. Its placement context supplies generation bounds,
world bounds, heightmaps and layer classifications from those same chunks.
`TerrainFeatureAccess` shares that region with feature algorithms and forwards
all writes and flags to a caller-supplied mutation callback.

Run `python3 tools/test_feature_region_parity.py`. The
[region report](../benchmarks/rust-feature-region-2026-10-06/scoped-parity.json)
records **28,922,634 matching lines per JAR** across 192 fixtures and 2,304 requested
placements. Actual native features and `FeaturePlacer` operate over a world proxy
backed by nine `ProtoChunk`s per fixture. The comparison includes both heightmap
groups, count/square/heightmap/layered chains, repeated writes, missing neighbors,
rejected writes, failures, update flags, random state, final raw sections and
packed heightmaps.

The region corpus also runs a composed sequence of configured fill layers over
the same live chunks: water, horizontal logs, air and an out-of-height layer.
This connects block-state decoding, nested feature execution, shared placement
and chunk heightmap updates; the standalone fill-layer suite below covers
integer-property parsing and per-column query ordering in more detail.
Single-block replacement and scattered ore also execute against those resident
chunks, sharing the updated state, heightmaps, failure policy and random stream.

The reference proxy supplies storage mutation. This verifies the connection
between features, placement queries and resident chunks; actual region write
zones, POIs, block entities, postprocessing, lighting and the full feature registry
remain pending. Unit tests check mixed-registry/bounds rejection, negative chunk
routing, missing-neighbor errors and mutation-callback flag/return handling.


## Inline feature composition

`FeatureDefinition` and `PlacedFeatureDefinition` compose the initial block-writing
features with no-op, sequence, boolean choice, uniform choice, chance-based
choice, weighted-choice and overlay features. Overlays execute every child and
accumulate success. Sequences stop on the first false child. Selectors return the
chosen child's result directly. Child modifiers and features share world state
and randomness; each sequence uses one fresh placer across its children.
Native single-object holder-set shorthand is accepted for sequence/uniform lists.

Run `python3 tools/test_feature_graph_parity.py`. The
[graph report](../benchmarks/rust-feature-graph-2026-10-06/scoped-parity.json) records
**1,146,057 matching lines per JAR** for actual native feature codecs and nested
placement. Cases include empty/malformed lists, singleton and nested choices,
zero/one probabilities, negative zero, false child results, partial writes, world
failures, repeated calls and top-level modifier chains.

Decode depth, node count and list size are bounded. Feature/placed-feature visits
and nested modifier-chain execution share a runtime budget. Unit tests cover deep
decoding, large trees and modifier expansion that reaches no leaf features.
Weighted choices retain zero-weight children and return false without random draws
when their total weight is zero. Positive totals preserve the existing bounded
weighted draw algorithm. `OptionalWeightedList` keeps this behavior separate from
the nonempty weighted lists required by integer/height providers. The graph suite
covers empty/zero lists, integer narrowing, invalid/overflowing weights and nested
choices; both provider suites also pass after the shared implementation change.
The resource resolver below adds named holders and tags. Encoders and the full
feature registry remain pending.


## Named feature resources

`FeatureCatalog` stores an immutable resource snapshot and resolves requested
feature or placed-feature roots. Named children, compact singleton holder sets,
mixed named/inline lists and ordered placed-feature tags expand into executable
trees. Resolution preserves numeric JSON fields, validates unselected children
and bounds dependency depth and expanded nodes. Duplicate canonical resource
names, missing dependencies and cycles return errors.

Run `python3 tools/test_feature_catalog_parity.py`. The
[catalog report](../benchmarks/rust-feature-catalog-2026-10-06/scoped-parity.json)
records **2,528,704 matching lines per JAR** against actual registry-aware holder
codecs, registered feature snapshots and native `TagLoader`. It compares ten
named features, fifteen placed features, five tag files, nested/duplicate references,
optional/required missing tags, namespace shorthand and subsequent execution.
Configured fill layers, single-block replacements and scattered ore bind block-state
metadata through named roots, nested tags and modifier chains. Block-set
predicates execute through filters, ordered volumes and environment scans against
the mutable world, including repeated placement after earlier writes and flowing-water replacement
that changes later fluid-predicate results.

Empty named tags retain a native distinction from empty inline lists: a sequence
can resolve an empty tag and succeed, an overlay returns false, while a uniform selector resolves it and
then fails its zero-bound random choice. The inline list codecs reject empty
lists during decoding. These cases and their random-state effects are compared.

The resource API validates requested reachable roots, allowing unused unported
resources to remain in the snapshot. Complete datapack loading, live reloads,
registry ownership/identity semantics and all feature algorithms remain pending.

## Configured fill layers

`FeatureBlockBindings` binds block-state JSON against the pinned registry and its
integer-property metadata. `FillLayerFeature` visits 16 by 16 columns in native
X/Z order, queries the world minimum for each column and replaces only air states.
It preserves wrapping coordinates, write flags, ignored false write results and
early termination on world errors. Placement consumes no random draws.

Run `python3 tools/test_fill_layer_parity.py`. The
[fill-layer report](../benchmarks/rust-fill-layer-2026-10-06/scoped-parity.json)
records **1,305,274 matching lines per JAR**, including codec inputs for all
35,723 pinned states, compact/full states, integer properties, invalid-property
defaults, height bounds, extreme coordinates, rejected writes and injected query,
read and write failures. Both random implementations retain matching subsequent
state. This comparison uses observable mutable storage; complete region effects
and full generated worlds remain pending.

## Replacement rules

`RuleTest` implements all ten pinned replacement-rule kinds: constant true,
negation, all/any, block identity, exact block state, block tags, inclusive height
range and the two random block/state matches. Logical rules preserve ordering and
short-circuiting. Random rules draw only after matching identity, including at
zero/one probability; their codecs allow probabilities outside that interval.

Run `python3 tools/test_rule_test_parity.py`. The
[rule report](../benchmarks/rust-rule-test-2026-10-06/scoped-parity.json) records
**1,890,716 matching lines per JAR**, covering every block/state identity, all
300 bundled block tags, absent and empty-path tags, malformed inputs, reversed
height ranges, nested logic and subsequent draws from both random streams.
Decode depth, node count and list size are bounded, including branching inputs.

Bindings share immutable sets of state IDs for blocks and resolved tags.
Native tag identifiers permit an empty path, which remains distinct from a
missing field. These predicates do not establish complete ore generation or
structure-processor behavior.

## Single-block replacement

`ReplaceBlockFeature` applies ordered rule/state pairs at an origin. It re-reads
the current state for each candidate and stops at the first match, regardless of
whether the write returns true. Empty or unmatched target lists still succeed;
read/write errors stop evaluation with the preceding mutations and RNG state
intact. Target decoding shares the graph's node budget when nested in a feature.

Run `python3 tools/test_replace_block_parity.py`. The
[replacement report](../benchmarks/rust-replace-block-2026-10-06/scoped-parity.json)
records **303,678 matching lines per JAR** for actual native codecs and placement,
including rule ordering, shared random state, update flags, rejected writes,
partial failures, repeated mutations and invalid unselected targets.

## Scattered ore

`ScatteredOreFeature` samples the native attempt count, consumes six float draws
for each offset (including zero spread), rounds with Java's tie behavior and
captures the candidate state once per attempt. Matching target rules then apply
the configured air-exposure probability and ordered directional neighbor checks.
The first accepted target writes with flag 2; a false write result is ignored.

Run `python3 tools/test_scattered_ore_parity.py`. The
[scattered-ore report](../benchmarks/rust-scattered-ore-2026-10-06/scoped-parity.json)
records **758,484 matching lines per JAR** across 11,544 fixtures and 23,088
requested placements. Cases include size/discard codec boundaries, empty targets,
solid/mixed worlds, each of the six air-neighbor directions, wrapping positions,
partial failures, rejected writes and both shared random streams.

Ordinary ore veins use a separate bulk-section mutation path and remain pending.
A private Linux/JDK 25 probe also found 4,935 bit differences among 2,581,112
double sine/cosine results for sampled ore angles when using Rust platform math.
Those intermediates are not yet validated for vein geometry; no exact double-trig
parity is claimed.
This scoped feature comparison does not establish biome decoration or full
generated-world parity.

## Bulk section access

`TerrainBulkAccess` exclusively borrows resident chunks and caches sections by
vanilla's packed section key. Raw reads retain cave/void air even when normal
chunk reads collapse an all-air section to AIR. Outside-height reads return AIR
before resolving a chunk. Raw writes maintain section occupancy but leave chunk
heightmaps untouched; region write permissions remain the caller's responsibility.

Run `python3 tools/test_bulk_section_parity.py`. The
[bulk report](../benchmarks/rust-bulk-section-2026-10-06/scoped-parity.json) records
**959,640 matching lines per JAR** against actual `BulkSectionAccess` and resident
`ProtoChunk`s. It covers every state, primed/lazy heightmaps, missing chunks,
vertical boundaries, packed-coordinate aliases, cache reset and complete final
section/heightmap data. The exclusive Rust borrow replaces Java's acquire/release
locks within this resident-region API; concurrent section ownership remains pending.

## Biome feature ordering

`build_features_per_step` combines ordered biome feature lists using placed-feature
object identities. It assigns encounter indices, orders graph traversal by step
and index, detects cycles and reverses DFS completion order to produce each step's
feature list and identity-to-index mapping. Equal feature payloads remain separate
when their identities differ. Traversal uses an explicit stack and bounded input
work; tests include a 50,000-feature dependency chain.

Run `python3 tools/test_feature_sort_parity.py`. The
[ordering report](../benchmarks/rust-feature-sort-2026-10-06/scoped-parity.json)
records **411,751 matching lines per JAR** across 3,871 fixtures. Native
`FeatureSorter` receives identity-distinct dummy payloads for synthetic graphs and
the feature lists from all 67 bundled biomes, including shuffled/subset combinations.
Cycles, duplicate references, cross-step reuse, empty steps and index lookups are
compared. Cycle diagnostic minimization/text and complete decoration remain pending.

## World-generation random streams

`WorldgenRandom` applies vanilla's legacy bit-sampling interface to legacy or
xoroshiro sources. Longs and doubles retain their multi-draw assembly, bounded
integers retain legacy rejection and each bit request increments a wrapping count.
Reseeding preserves the counter. Forks and positional factories delegate to the
underlying source and return its normal draw behavior through a transparent variant.

Run `python3 tools/test_worldgen_random_parity.py`. The
[random-adapter report](../benchmarks/rust-worldgen-random-2026-10-06/scoped-parity.json)
records **238,800 matching lines per JAR** for 119,100 operations in 300 fixtures.
It includes one to three nested wrappers, arbitrary signed bit-width shifts,
failed bounds, parent/child streams, positional UTF-16 hashes and decoration,
feature, large-feature, salted and slime-chunk seeds with int/long overflow.
Gaussian sampling, concurrent use and complete chunk decoration remain pending.

## Biome placement filtering

`Biome` requires the invocation's top placed feature before querying the biome and
its generation settings. `place_with_biome_check` establishes that context for
the current modifier chain; a nested ordinary placer starts without it. Biome
resolution and membership are supplied by the world. Native membership treats
structurally equal placed-feature copies as equal, while feature ordering uses
object identities.

Run `python3 tools/test_biome_placement_parity.py`. The
[biome-filter report](../benchmarks/rust-biome-placement-2026-10-06/scoped-parity.json)
records **121,423 matching lines per JAR** for 27,456 requested placements across
9,152 fixtures. Actual `BiomeFilter` and `FeaturePlacer` compare missing roots,
equal copies, different features, ordered queries, nested ordinary placement,
injected failures and repeated calls with retained buffers. Both random sources
are tested directly and through `WorldgenRandom`. Complete registry membership,
biome sampling integration and chunk decoration remain pending.

## Decoration feature scheduling

`FeatureDecorationPlan` owns immutable biome generation lists and their globally
ordered step features. It reads the surrounding nine chunks in X-first order,
intersects their resident biomes with the generator's possible-biome set and
executes the sorted union of global feature indices for each step. Each root
receives its decoration/feature seed and a shared placer. False results do not
skip subsequent roots. Errors retain previous work and the current-feature
marker; successful completion clears the marker.

Run `python3 tools/test_decoration_parity.py`. The
[decoration report](../benchmarks/rust-decoration-2026-10-06/scoped-parity.json)
records **337,398 matching lines per JAR** across 2,188 fixtures and 4,376 requested
decoration calls. Actual `ChunkGenerator.applyBiomeDecoration` runs with
structures disabled, resident `ProtoChunk` biome palettes and traced feature
sinks. All 67 bundled biome feature lists participate. Sparse selections,
excluded biomes, step gaps, cycles, coordinate overflow, biome-check modifiers,
false returns and query/placement/progress failures are compared. Rust bounds
extreme signed chunk neighborhoods and input work explicitly. Structure
decoration, real feature payloads in this suite and full chunk lifecycle remain
pending.

## Decoration over resident chunks

`BiomeRegion` connects quart biome sections to seeded block-position biome zoom.
`TerrainRegion::biome_at` checks the selected chunk's generation stage before
reading its biome. Premature reads fail even if raw palette values already exist;
decoration's initial palette collection can still inspect those raw values.

Run `python3 tools/test_decoration_region_parity.py`. The
[resident decoration report](../benchmarks/rust-decoration-region-2026-10-06/scoped-parity.json)
records **36,173,951 matching lines per JAR** for 240 fixtures and 480 requested
decoration calls. Java's actual decoration entry point places three basic
features, configured fill-layer sequences, replacements and scattered ore into
nine `ProtoChunk`s. Uniform/mixed biome palettes, biome zoom, global scheduling,
feature seeds, biome filters, named biome-set and volume predicates, live
heightmaps, repeated decoration and rejected
or failed writes execute together. Every final raw block and heightmap word is
compared, including EMPTY, BIOMES and TERRAIN stage behavior. Workspace tests,
Clippy and 137 Python tests passed after this integration.

The world transport supplies storage mutation. Actual `WorldGenRegion` write
zones, retrogen, POIs, block entities, postprocessing, lighting, structures and
the complete noise-to-decoration lifecycle remain pending.

## Block-state providers

`BlockStateProvider` supports simple states, weighted states, random blocks,
rotation, property copying, randomized integer properties, three noise-based
providers and rule-based selection. Direct decoding preserves the exclusive choice
between a full state object and a typed provider. Random block choices retain
holder/tag order and duplicates. Empty choices return no optional state, while
ordinary sampling reads the existing world state. Rotation chooses a direction
before sampling its nested provider, changes only compatible axis/facing
properties and preserves other properties. Fixed directions consume no direction
draw. Property copying preserves only compatible property families. Integer
sampling leaves states without the named integer property unchanged without
drawing a value; invalid sampled values fail after consuming the draw. Shared
compact state layouts avoid rebuilding property maps during these transforms.

Noise providers use independently seeded legacy noise and preserve the native
float/double coordinate conversions. Threshold selection preserves conditional
random draws. Dual noise computes the selected candidate directly: the other
candidates are pure noise queries without random or world side effects.
Rule-based providers evaluate conditions in order, continue after empty optional
results and sample their fallback only after all rules are exhausted. Current
conditions cover true, not, all-of, any-of, world bounds, height ranges, block tags
volume matching and registry-bound block/fluid/biome sets. Height-range predicates reconstruct generation bounds per
evaluation; volume matching visits X, Z, then Y and stops on the first failure.
World adapters supply tag membership and fallible generation-bound queries. The
height-range comparison currently exercises ordinary LevelAccessor bounds; the
WorldGenLevel server/generator lookup branch is not covered by this fixture.

Interrupted runs can be resumed with `python3 tools/test_state_provider_parity.py --resume`.
The runner verifies source, environment, fixture and compiled-artifact hashes
before reusing completed stages. A changed output is regenerated; changed inputs
require a fresh run without `--resume`. Checkpoints stay under `.local`, and
a parity report is written only after both native comparisons pass.

Run `python3 tools/test_state_provider_parity.py`. The
[provider report](../benchmarks/rust-state-provider-2026-10-06/scoped-parity.json)
records **257,299,719 matching lines per JAR**, with 356,935 codec inputs
and 79,473,574 requested samples across ten provider kinds. Every pinned state is decoded and tested
under all six fixed rotations; random/nested rotations, both sampling methods,
empty tags, weighted boundaries, fallback query failures and four random-stream
combinations are compared. Every target state is also tested against
representatives of all compatible property values, including cross-block copies
and integer-property boundaries. The native harness verifies that the pinned
property-family signatures are unambiguous; Rust validates the Cartesian state
tables before binding their transitions. Noise cases include all eight bundled
provider definitions, coordinate boundaries, extreme scales, variety
selection and malformed codec inputs. Rule cases cover ordered empty results,
fallbacks, nested rules, bound-query failures and wrapping coordinates. Block-tag
cases cover every tag against all block defaults and the two bundled rule-based
definitions against every state. Volume comparisons include exact read order and
early exits, while Rust-only tests bound pathological nested evaluation work. Workspace
tests, Clippy and 137 Python tests passed at this stage.

The unobstructed predicate validates its optional integer-vector offset but
ignores it during evaluation, matching the pinned Java implementation. The
comparison checks exact translated unit-block bounds, a null excluded entity,
true/false world results, nested volumes and query failures.

`BiomePredicateBindings` resolves biome names and holder tags to IDs in the
world's owning registry snapshot. The comparison covers every pinned biome and
every bundled biome tag, empty sets, missing names, query failures and nested
volume/short-circuit conditions. Decoration shares the same `BiomeId` identity.

`FluidStateTable` derives fluid identity, amount, falling/source flags and own
height from the pinned block descriptors. The harness compares every state,
including falling source water in all eight waterlogged copper grates. Fluid
predicates compare all five fluid kinds, holder lists and every bundled fluid tag
against every block state. This covers contained fluid state, not spreading or
entity interactions.

`StateProviderCatalog` resolves named holder references from an immutable resource
snapshot. Holder, direct and typed entry points retain their distinct decoding
semantics, including ambiguous state/provider objects. The comparison registers
all eight bundled definitions plus nested fixture resources and checks missing
names, nested references and later random draws. Unused resources are not
evaluated; Rust tests reject reachable cycles, duplicate canonical names and
excessive expansion. Full datapack registry loading/reloading remains separate.

Recursive provider decoding is bounded to depth 128 and shares a 65,536-node
budget with rules, predicates and integer providers; holder/rule lists are bounded
to 8,192 entries. Additional predicates, complete registry loading, block survival
and complete feature placement remain pending.

Solid and replaceable predicates now bind immutable property tables to the same
block registry as provider and feature decoding. Their offsets, state reads,
short-circuit composition, empty optional-child fallthrough and read failures
are compared across the entire state registry. Together with the earlier kinds,
**14 block-predicate kinds** are covered in the state-provider report. Full
workspace tests, warning-free Clippy and **139 Python tests** passed at this
integration snapshot. Face support and block survival still need implementation.

The has-sturdy-face predicate now decodes native directions and bounded offsets,
then delegates the ordered state/support query to its placement world. The
state-provider comparison covers all **35,524 cached states**, six directions,
read failures and composed predicates, bringing the scoped corpus to **15
predicate kinds**. Dynamic state support is not established by this integration
report; unavailable world behavior remains an explicit query error. Workspace
tests, warning-free Clippy and **140 Python tests** passed at this snapshot.

Unified block-support dispatch now extends that predicate corpus to all **35,723
states**, including the **199 dynamic states** with absent block entities and
injected block-entity query failures. The updated provider report covers the
combined path. Shulker animation snapshots are covered separately by the dynamic
support suite; active moving-piston shape generation and block-entity lifecycle
remain outside this integration snapshot. Workspace tests, Clippy and **141
Python tests** passed.

The `would_survive` predicate now decodes a candidate block state and bounded
offset, then invokes survival at the translated position without placing it.
Both provider and feature bindings use the shared block-state codec; placement
chains forward the survival query. The refreshed provider report records
**257,299,719 matching lines per JAR**, **356,935 inputs**, **79,473,574 requested
samples** and **16 predicate kinds**. Its survival snapshot now covers all
**51 pinned survival families, 1,286 definitions and 35,723 states**. Behavior
representatives run against the entire neighbor-state registry, with malformed
definitions, wrapped offsets, brightness thresholds, short-circuit conditions,
optional-child fallthrough and query failures. Dynamic support and empty-context
collision use absent block entities in this integration corpus; animated shulker
and active-piston snapshots have separate geometry comparisons. This establishes
the recorded provider/predicate integration, while full block placement, updates
and gameplay remain pending. Workspace tests, warning-free Clippy and the
**147-test Python suite** passed.


## Simple-block placement

`SimpleBlockFeature` combines holder-aware state providers, candidate survival,
precompiled placement transitions and optional scheduled ticks. Ordinary writes
retain native flags and ignore a rejected write's boolean result. Double plants
first check the block above for air or matching replaceable fluid, then copy
waterlogging independently into each half. Pale moss carpet derives side states
from support/collision queries, consumes the world's separate random stream for
its topper and recomputes lower sides after the upper write. Scheduled ticks use
the block actually stored after placement, including rejected writes.

`FeatureBlockBindings::with_state_providers` binds these operations and provider
resources to the same registry. `FeatureDefinition` and `FeatureCatalog` compose
simple blocks with named holders, selectors, sequences and placement modifiers.
Provider decoding shares the enclosing graph's bounded work/depth budget; compiled
placement tables are shared by immutable reference-counted handles.

Run `python3 tools/test_simple_block_parity.py` against the pinned JARs. The
[simple-block report](../benchmarks/rust-simple-block-2026-10-06/scoped-parity.json)
records **9,613,872 matching output lines per JAR** across **492,554 fixtures**.
It covers every candidate state, double-plant obstructions, waterlogging, moss
attachment/topper combinations, ordered reads/writes, both random streams,
rejected writes, partial failures, wrapped coordinates and nested composition.
Workspace tests and all-target Clippy passed. Native chunk updates, scheduled tick
execution and complete playable integration remain pending.


## Block columns

`BlockColumnFeature` samples all layer heights before checking space. Its first
obstruction query is one step beyond the origin. It removes excess height from
the base or tip in native order, then evaluates providers while writing the
remaining layers from the original position. Empty optional providers use their
ordinary fallback read; rejected writes still advance the position. A completely
zero-height sample returns false, while obstruction that trims every layer to
zero still returns true.

The [block-column report](../benchmarks/rust-block-column-2026-10-06/scoped-parity.json)
records **1,204,081 matching lines per JAR** over **64,120 fixtures**. Run
`python3 tools/test_block_column_parity.py` to compare all candidate descriptors,
six directions, both truncation priorities, provider and predicate composition,
wrapped coordinates, codec bounds and partial mutations. This uses the same
feature/catalog integration and mutable-world harness as simple-block placement.
Layer/decode limits and a million-operation runtime budget reject pathological
workloads explicitly; complete resident-world integration remains pending.


## Vertical pillars

The single-block pillar feature checks its replacement predicate, consumes its
continuation draw, and then checks build height before evaluating and writing
each provider state. Its cap runs one step opposite the stopping position,
including when no pillar block was written. Rejected writes still advance the
pillar, and a cap's false result does not make the pillar return false. Named caps
and nested pillars use the same feature catalog and graph work budget.

Run `python3 tools/test_pillar_parity.py`. The
[pillar report](../benchmarks/rust-pillar-2026-10-06/scoped-parity.json) records
**921,111 matching lines per JAR** across **41,746 fixtures**, including every
candidate descriptor, both vertical directions, continuation boundaries,
signed-zero probability rejection, replacement conditions, cap composition,
failures, rejected writes and random
stream tails. Workspace tests and all-target Clippy passed. Resident world
updates and complete generation/gameplay remain unfinished.


## Neighbor-based spreading

The random-neighbor-spread feature evaluates and writes its origin provider before
sampling attempts or testing replacement. Each attempt samples X, Y and Z offsets
in order, checks replacement, then visits neighbors in native direction order.
Exactly one accepted neighbor allows placement; finding a second stops the scan.
Rejected writes preserve the same subsequent queries and random consumption.

The [neighbor-spread report](../benchmarks/rust-neighbor-spread-2026-10-06/scoped-parity.json)
records **2,650,856 matching lines per JAR** across **79,580 fixtures**. Run
`python3 tools/test_neighbor_spread_parity.py` to compare all candidate and neighbor
states, all direction masks, bounded integer providers, named/empty holder sets,
mutable repeated attempts, partial failures and nested feature/modifier calls.
Workspace tests and all-target Clippy passed. These feature algorithms still
require complete chunk mutation, neighbor updates and gameplay integration.

## Additional verified feature algorithms

The executable feature graph also includes overlay composition, block piles,
disks, block blobs, springs, vines and underwater magma. Their version-specific
reports and remaining integration limits are recorded in [Rust rewrite status](rust-parity.md#overlay-feature-composition).
These comparisons cover ordered world requests; complete world updates and
playable gameplay remain pending.
