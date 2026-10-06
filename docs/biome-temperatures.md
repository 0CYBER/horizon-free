# Biome temperatures

Horizon implements Java Edition **26.3** biome temperature sampling, including the
frozen modifier, height adjustment, rain/snow thresholds, precipitation selection
and the small frozen-ocean iceberg melt predicate. The fixed seeded simplex and
noise-stack sources use the existing Rust noise implementation.

`BiomeTemperatureSettings` supplies the immutable inputs. `BiomeTemperature` is a
per-biome, per-worker cache, corresponding to vanilla's thread-local cache. Its
1,024-entry insertion order does not change on hits. Keys use packed block
positions and omit sea level, preserving both coordinate aliases and cached
results across sea-level changes. NaN values are recomputed, including vanilla's
eviction behavior when a cached NaN is queried at capacity.

Run `python3 tools/test_biome_temperature_parity.py`. The
[local report](../benchmarks/rust-biome-temperature-2026-10-06/scoped-parity.json)
records **836,659 matching output lines per reference JAR** over **387 fixtures**,
including **12 thread-isolation fixtures**. The original Java transport invokes
actual biome methods and inspects native ordered cache contents. Test inputs
include arbitrary base-temperature bits, threshold neighbors, both modifiers,
precipitation enabled/disabled, extreme coordinates, sea-level overflow, packed
aliases, cache hits/eviction and actual native threads with separate Rust workers.

The oracle constructs native climate settings and biomes with only the fields
these methods use. Climate-setting parsing now covers all **67 bundled biome definitions** and
**183 codec inputs**, including float rounding, overflow, field validation and
modifier defaults. Full biome codecs, climate encoding, world-dependent snow/ice placement,
automatic thread-local ownership for a shared
Rust biome object remain pending. Source/fixture hashes guard the comparison;
raw reference artifacts stay private under `.local`.

[Material temperature and biome conditions](material-conditions.md) now use these
climate settings and cached temperature calculations in a separate integration suite.
