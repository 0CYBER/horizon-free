# Biome lookup

Horizon implements Java Edition **26.3** block-to-quart biome lookup. `BiomeZoom`
accepts the obfuscated biome seed and chooses among eight jittered quart corners
in vanilla order, retaining strict distance ties and signed coordinate wrapping.
The caller resolves the selected quart coordinate through its biome provider.
Direct noise-biome queries bypass this selection and shift block coordinates;
floating-point positions are floored with Java's saturating integer conversion.

`obfuscate_seed` computes the vanilla SHA-256 transform of the little-endian world
seed and reads the first eight digest bytes as a little-endian signed long. The
core keeps a fixed-input implementation without adding runtime dependencies.

Run `python3 tools/test_biome_zoom_parity.py`. The
[local report](../benchmarks/rust-biome-zoom-2026-10-06/scoped-parity.json) records
**256,933 matching lines per reference JAR**: **4,104 seed hashes**, **169,408 block
queries** and **83,421 floating-position queries**. Actual vanilla `BiomeManager`
methods call a traced resolver; the suite compares selected quart coordinates,
direct queries and source replacement retaining the same zoom seed. Seed hashes
also match Python `hashlib` independently.

The corpus includes dense neighborhoods, negative quart boundaries, both signed
integer extremes, distant coordinates, randomized seeds/positions, and double
NaNs, infinities and subnormals. Source and fixture hashes are captured, changes
during validation invalidate the result, and reference artifacts stay private.
CI includes the comparison; remote outcomes have not been observed.

This validates coordinate selection, not biome storage, biome-source generation,
chunk blending, or complete generated worlds.


## Chunk biome storage

`BiomeChunk<T>` stores each section's 64 quart-resolution biome values. Reads clamp
quart Y to the chunk and wrap quart X/Z into its local grid. Fills visit sections
bottom to top and query each section in X/Y/Z order. Each completed section is
committed; a resolver failure retains the failing section's previous values.
Inclusive generation bounds select all intersecting sections.

Run `python3 tools/test_biome_chunk_parity.py`. The
[local chunk report](../benchmarks/rust-biome-chunk-2026-10-06/scoped-parity.json)
records **478,340 matching lines per reference JAR** across **20 fixtures** and
**160 fills**, including **21,120 quart-biome queries**. Native `ProtoChunk` and
`LevelChunkSection` methods supply the reference behavior. Tests use synthetic
holder identities, up to 512 distinct values, repeated fills, provider failures
at calls 0/63/64/65, generation bounds cutting through sections, and the complete
supported aligned vertical range. The reference chunk is at BIOMES status.

This storage uses dense arrays and compares biome values rather than vanilla's
internal palette layout. Packet/save integration and chunk lifecycle transitions
remain separate work. Clipped generation bounds must remain within the chunk;
invalid bounds return an error before mutation.


## Chunk biome generation

The [generation report](../benchmarks/rust-biome-generation-2026-10-06/scoped-parity.json)
records **383,072 matching lines per JAR** across **252 fixtures**, **322 fills**,
**341,504 stored biome values** and **35,910 subsequent block-position lookups**.
Run `python3 tools/test_biome_generation_parity.py` to compare against actual
`ChunkGenerator.doCreateBiomes`, invoked through `NoiseBasedChunkGenerator`.

Rust compiles the climate fields of all seven bundled terrain routers, samples
the chunk's quart volume, searches its multi-noise parameter table and fills
biome sections. The suite pairs each router with both Overworld and Nether
presets, both random modes and three seed/coordinate combinations. Repeated
fills create fresh density contexts while retaining nearest-search state.
Generation bounds also exercise omitted top and bottom sections. Subsequent
block lookups use seeded fuzzy selection and clamped chunk reads.

Both JARs independently export the preset tables and must agree: **7,594 Overworld
entries** and **5 Nether entries**. Rust imports those quantized tables; preset
builders remain unported. Registry-bound source codecs are described below. Reference holders
supply identities without evaluating biome properties. That baseline uses empty
blending and no below-zero retrogen. An additional 168 fixtures generate chunks through fixed, checkerboard and End
sources using every bundled terrain router. They cover real seeded End erosion,
the central-island boundary and wrapped checkerboard shifts. The [surface-stage integration](material-surfaces.md#chunk-surface-stage) now
links a 3×3 biome neighborhood to terrain filling and surface application, using
stored neighbor biome lookups. Saved biome palettes and complete world generation
remain pending.


### Below-zero biome retention

`RetrogenBiomeResolver` captures whether an upgrade is active when the resolver
is created. Later metadata changes do not add or remove its retention policy.
It always samples the candidate first. Keyed `lush_caves`, `dripstone_caves` and
`deep_dark` in the `minecraft` namespace are retained; other candidates read the
old biome at the same quart X/Z and quart Y = 0. Unkeyed holders do not qualify.

`BiomeChunk::fill_with_retrogen` reads committed old sections while staging the
current section separately. A failure preserves that section's old values;
later sections can observe earlier completed writes. This matters when supplied
generation bounds overlap the section selected by the clamped old-column read.

The [retention/fill report](../benchmarks/rust-retrogen-biome-fill-2026-10-06/scoped-parity.json)
records **593,152 matching lines per reference JAR**, with 40 fixtures, 740 fills
(560 decorated), and 2,800 direct resolver queries. It includes all activation
timings, keyed and direct holders, foreign namespace lookalikes, callback failures,
boundary sections and extreme coordinates.

The expanded [generation report](../benchmarks/rust-retrogen-biome-generation-2026-10-06/scoped-parity.json)
records **834,080 matching lines per JAR** across **396 fixtures and 610 fills**.
Its 144 added upgrade fixtures use actual native below-zero metadata and generation
heights, patterned old biome columns, all source kinds, each retained cave biome,
four settings with below-zero storage, both random modes and repeated fills.
Every stored section and subsequent block-biome lookup is compared, including
unchanged data above Y = 0. Nonempty blending, preset builders, full terrain-stage
scheduling and complete saved-world integration remain pending.

## Fixed, checkerboard and End sources

`FixedBiomeSource` always returns its configured value. `CheckerboardBiomeSource`
preserves masked Java shifts, wrapped addition and floor-modulo selection,
including duplicate values and unrestricted constructor scales. It supports up to
8,192 values; an empty list returns a typed lookup error. Its vanilla JSON codec
has the narrower scale range 0–62, with a default of 2.

`EndBiomeSource` returns the central biome without querying erosion inside the
64-chunk radius. Outside that circle it samples erosion at the containing chunk's
center and wrapped block Y, then applies vanilla's highlands/midlands/islands/
barrens thresholds. Sampler errors propagate without substituting a biome.

Run `python3 tools/test_biome_source_parity.py`. The
[local source report](../benchmarks/rust-biome-source-2026-10-06/scoped-parity.json)
records **1,306,096 matching lines per JAR**, covering **457 fixtures** and
**1,147,984 queries**. Actual native source methods supply the results. The corpus
includes all 63 codec-valid checkerboard scales, unrestricted runtime scales,
empty/duplicate/512-value lists, extreme coordinates, exact End threshold
neighbors, NaN/infinity samples and failing erosion providers. Ordered erosion
sampling coordinates are compared. This source-level suite supplies constant
End density values; seeded density and chunk generation are validated separately.


## Registry-bound source codecs

`BiomeSourceCatalog` decodes and canonically encodes fixed, checkerboard, End and
multi-noise definitions against known biome, tag and preset registries. Named
biomes, direct or tagged biome sets, inline climate lists, and named or inline
preset holders retain vanilla selection and fallback behavior. The catalog
validates references before returning a definition.

`BiomeSourceDefinition::resolve` builds a `ResolvedBiomeSource` using imported
quantized preset tables. A worker resolver retains multi-noise search state,
prepares climate volumes only for multi-noise sources, and exposes one query API
for all four source kinds. Chunk generation and the surface-stage neighborhood
now use this API; surface integration decodes the source configuration itself.

Run `python3 tools/test_biome_source_codec_parity.py`. The
[local codec report](../benchmarks/rust-biome-source-codec-2026-10-06/scoped-parity.json)
records **363,121 matching lines per JAR** across **40,481 inputs**, using all
**67 biome definitions** and **86 biome tags**. The native `BiomeSource.CODEC`
supplies parsing, canonical encoding, possible-biome sets and queries at seven
positions. The corpus covers every quantized climate step from −2 to 2, scale
bounds and numeric narrowing, malformed fields, duplicate keys, holder sets,
and inline-list preference and fallback to presets. Encoding comparisons use
serialized JSON numeric values, rather than Java's widened float values.

Full inline biome definitions, native preset builders, live registry ownership
and reloads, biome-search helpers, blending, and complete generated worlds remain
pending. The bounded catalog is not a complete registry lifecycle implementation.
