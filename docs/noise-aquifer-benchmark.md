# Bulk density and noise generation benchmark

The PR dispatches unary and binary density operations once per bulk buffer, resolves the shared Beardifier kernel once per sampled position, and reuses Perlin X-column permutations across the Y loop. Binary bit-exact tests cover every operation, distinct NaN payloads, signed zeros, infinities, subnormals, random float pairs and SIMD/tail-sized buffers. Child-failure tests retain left/right evaluation order and partial output. Scalar shortcuts, arithmetic expressions, RNG consumption and generation traversal are preserved.

## Whole-terrain measurements

**Times below are complete terrain wall elapsed divided by 256 input chunks, with no thread multiplier.** Four-worker figures are elapsed latency/throughput measurements, not per-thread CPU costs. Combined time adds the complete decoration phase, still divided by 256; 196 interior chunks are decorated with unchanged border work. At the primary site, 256 terrain chunks take about 2.578 seconds with one worker versus 0.860 seconds with four: approximately 3.00× throughput, or 75% of ideal fourfold scaling. A subsequent 1/2/4/8-worker scaling experiment is reported below.

Fresh equivalent optimized builds compare baseline `e4f8154cb4115aa1ee622381d9b12387cdf515d4` with the extended candidate. Every case has one discarded process pair, then five measured pairs alternating baseline/candidate order. Initialized processes first generate and discard the same offset 9×9 region at center+(64,64), using the same seeded generator, before timing fresh measured workers. This excludes lazy ring initialization without pre-generating target chunks. All cases use size 16; primary seed/site is 12345/(0,0), second Overworld seed/site is -1/(32,-32).

Medians below are baseline → extended candidate, ms/input chunk, followed by percent reduction.

- **overworld, 1 worker(s):** terrain 12.685400 → 10.069365 (20.62%); combined 14.696298 → 12.105007 (17.63%).
- **overworld, 4 worker(s):** terrain 4.045564 → 3.359536 (16.96%); combined 4.663124 → 3.944952 (15.40%).
- **the_nether, 1 worker(s):** terrain 5.088375 → 4.848149 (4.72%); combined 5.481617 → 5.239165 (4.42%).
- **the_nether, 4 worker(s):** terrain 1.446298 → 1.366468 (5.52%); combined 1.574890 → 1.472913 (6.48%).
- **the_end, 1 worker(s):** terrain 1.085323 → 0.980046 (9.70%); combined 1.161651 → 1.053080 (9.35%).
- **the_end, 4 worker(s):** terrain 0.281733 → 0.259724 (7.81%); combined 0.312280 → 0.288982 (7.46%).
- **overworld, 1 worker(s), second site:** terrain 11.585829 → 9.079823 (21.63%); combined 12.650823 → 10.145760 (19.80%).
- **overworld, 4 worker(s), second site:** terrain 3.105819 → 2.519504 (18.88%); combined 3.410155 → 2.812551 (17.52%).
- **overworld, 1 worker(s), non-initialized generator:** terrain 23.429316 → 20.942006 (10.62%); combined 25.599745 → 23.005476 (10.13%).

The non-initialized case retains lazy startup work and is separate from recurring-cost qualification; resource loading is outside all terrain brackets. All 45 measured terrain pairs improved, but Nether one-worker median terrain improvement remains below 5%. Results are workload-dependent. The primary one-worker noise bracket improves 5.901774 → 4.773963 ms/chunk (19.11%); shared bulk material-density work also improves, with surface 4.576660 → 3.181641 (30.48%). Whole-terrain gains must not be described as noise-stage-only gains. Parallel stage brackets are aggregate worker elapsed and are not exclusive CPU shares.

## Added benefit beyond unary dispatch

A separate five-pair alternating comparison uses the existing unary-only PR revision `594b50278044df6b72b641cd02d5050925c86203` as baseline. It uses the same initialized primary workload and fresh builds. These numbers measure the binary/Beardifier/Perlin additions together; they do not assign a separate speedup to each addition.

- **1 worker(s):** terrain 10.913267 → 10.034900 (8.05%); combined 12.867017 → 12.079432 (6.12%).
- **4 worker(s):** terrain 3.536831 → 3.312338 (6.35%); combined 4.110268 → 3.897104 (5.19%).

All ten incremental terrain pairs improved. Pair reductions ranged 4.72–9.93% with one worker and 2.10–8.61% with four, so not every individual pair exceeds 5%. A preliminary gate was excluded in its entirety because a concurrent Horizon client consumed roughly seven cores. Its raw samples remain in `contended-gate/`; all qualification runs began after the client was closed. No individual samples were selectively removed from the restarted runs.

## Worker scaling follow-up

Candidate revision `c47b30fcf0db87b8023e6784d941a4f06524b453` was tested at 1, 2, 4 and 8 workers on the same primary initialized Overworld workload. The saved `final-candidate` optimized binary is used without rebuilding or changing instrumentation. After one discarded process per worker count, eight measured processes per count run sequentially. Worker orders `[1,2,4,8]`, `[2,8,1,4]`, `[4,1,8,2]`, `[8,4,2,1]` repeat twice, placing each count in each order position twice. Every process initializes its generator using the same offset 9×9 region before measuring 256 terrain chunks and 196 interior decorations. No baseline comparison or new correctness claim is inferred from this scaling experiment.

| Workers | Terrain median ms/chunk (min–max) | Terrain chunks/s | Speedup | Efficiency | Combined median ms/chunk | Combined speedup |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 9.994644 (9.842544–10.312603) | 100.05 | 1.000× | 100.0% | 11.950737 | 1.000× |
| 2 | 5.580046 (5.499388–5.772284) | 179.21 | 1.791× | 89.6% | 6.644499 | 1.799× |
| 4 | 3.405281 (3.238112–3.525516) | 293.66 | 2.935× | 73.4% | 3.996531 | 2.990× |
| 8 | 2.750915 (2.662450–2.915874) | 363.52 | 3.633× | 45.4% | 3.220818 | 3.710× |

Speedup is the one-worker median divided by the respective median; efficiency divides speedup by worker count. Eight workers give 23.8% more terrain throughput than four workers, with 19.2% lower wall ms/chunk. This is sublinear scaling on this machine and site; eight workers exceed the six physical cores, and this experiment does not identify the causes of diminishing returns. Ordinary desktop activity was not isolated. All 32 measured runs are retained. Raw samples, per-process logs, binary SHA-256, protocol, runnable script and combined ranges are in `/srv/bob-data/horizon-noise-aquifer-e477fa56/extension/scaling/` (`raw.json`, `summary.json`, `protocol.json`, `run.py`). Run with `python /srv/bob-data/horizon-noise-aquifer-e477fa56/extension/scaling/run.py`; the script uses the prepared resources and saved optimized binary, requiring no build. The earlier comparison medians above remain unchanged.

## Machine and validation

AMD Ryzen 5 5600X, 6 cores/12 logical CPUs, 32 GiB RAM, Linux, Rust 1.99.0. Optimized `fast` profile: release optimization, LTO disabled, 16 codegen units. Builds and timed processes were sequential; ordinary desktop activity was not isolated. Primary terrain process CPU medians are 12.4 → 10.0 ms/input chunk (one worker) and 15.9 → 13.2 (four), using coarse 100 Hz accounting. Peak RSS is effectively unchanged: 786288 → 786256 KiB and 833496 → 833528 KiB. No new heap buffer is introduced. Executable text increases 1804 bytes against the original baseline and decreases 1056 bytes against the unary-only revision.

Validation of the extended candidate passed:

- 193 targeted optimized Rust unit/integration tests, including binary floating-point and failure-order tests, density/world fields, storage/palettes, material/carving stages, biome regions, retrogen and ticks.
- Fresh pinned Java client/server density parity: 15,524,627 output lines per side.
- Dedicated noise parity: 1,443,539 lines per side, covering Perlin/smeared bulk paths, strided/wrapped volumes, buffer tails, constructor RNG and derivatives.
- Dedicated Beardifier/structure-density parity: 183 fixtures, 1,360,095 lines per side.
- Terrain-fill parity: 114 fixtures, 200 fills, 7,950,873 lines per side, including prefilled chunks, repeated fills, partial vertical volumes, aquifers, WG heightmaps and ordered postprocessing.
- 24/24 exact region phase comparisons: three dimensions × two seeds/sites × one/four workers, each with 81 retained chunks and 49 interior decorations. Every retained chunk is compared before and after decoration, including borders. Snapshots include raw/public block reads and palettes, quart biomes, entities/block entities as binary NBT, complete scheduled ticks, all six heightmaps, ordered postprocessing, actual non-air counts and chunk metadata. Temporary diagnostic accessors were removed from production source.

This validates the declared corpora and sites, not every possible seed, biome or platform. Lighting, meshing, saving, streaming and full application startup were not benchmarked.

The existing `crates/horizon-world/examples/chunkgen_bench.rs` is the standard workload entry point. Both performance builds used the same saved diagnostic source adding optional generator initialization and exact elapsed/count output; its thread-multiplied human display was not used. Diagnostic builds use `CARGO_TARGET_DIR=/srv/bob-data/horizon-stage3d-target cargo build --profile fast -p horizon-world --example chunkgen_profile`. Raw timings, exact commands, binary/source hashes, scripts and parity outputs are retained outside Git at `/srv/bob-data/horizon-noise-aquifer-e477fa56/extension/`: `gate-raw.json`, `matrix-raw.json`, `summary.json`, `marginal/marginal-raw.json`, `binary-hashes.json`, and the `*-parity.json` reports. Prepared resources are `/srv/bob-data/horizon/.local/horizon-client/26.3`. Generated artifacts and private fixtures remain outside the PR.

The repository quick PR self-check also passes: public-tree validation, workspace rustfmt, workspace/all-target clippy with warnings denied, and 96 full core-crate tests. Relevant pinned Java suites were run separately as listed above. A production-only rebuild after removing diagnostic support has byte-identical executable instructions and read-only constants to the measured candidate.
