# Aquifer neighborhood cache benchmark

Caching the most recently used aquifer grid neighborhood reduces complete Overworld proto-generation plus decoration by a median **4.152% with one worker** and **2.377% with four workers** at the primary site. A second site improves **3.865% / 3.275%**. All 20 measured Overworld pairs improve. The implementation changes only `worldgen/aquifer/noise_based.rs`.

## Implementation and scope

Keep the twelve decoded cell positions and indices for the current grid in vanilla traversal order. Publish a replacement neighborhood only after every checked grid access succeeds; location entries initialized before an error remain visible in their original order. Reuse the neighborhood while its grid key matches. The existing location/status/surface caches retain their behavior. Explicit four-slot nearest insertion preserves later-cell distance ties (`<=`), wrapping distance arithmetic, random draw order, sampler/error order and fluid-update scheduling. Pinned 26.3 `Aquifer.NoiseBasedAquifer.computeSubstance` bytecode was inspected before implementation.

No overlap with open aquifer work was found on 2026-10-22. The excluded Perlin, climate, feature-region, ore, biome-temperature and chunk-bookkeeping mechanisms are unchanged. Baseline `753c071c` includes the accepted saved-palette optimization; both variants contain that same loading code. These generation results measure only the aquifer change.

## Measurements

AMD Ryzen 5 5600X, 6 cores/12 logical CPUs, 32 GiB RAM; Linux 7.2.11-zen1-1-zen; Rust 1.99.0; release optimization, thin LTO, one codegen unit. Boost stayed enabled with the powersave governor and ordinary desktop activity. Resources: `/srv/bob-data/horizon/.local/horizon-client/26.3`. Both executables were rebuilt from accepted sources with identical benchmark source (SHA256 `f404feb7ec526f12735d11b15af36254fd585f39b975cdbdc71905d87f10f8cc`), dependencies, resources and flags. Builds, tests and profiling did not overlap qualification timing.

Every row uses one discarded process pair, then five measured pairs alternating baseline/candidate order. All 84 processes produced 256 proto chunks and 196 interior decorations. Warmup generates/discards an offset 9×9 region before fresh measured workers. Primary seed/center: 12345/(0,0); second: -1/(32,-32). Values are process-median **milliseconds for the complete 256-input-chunk batch**, without a thread multiplier. Stage medians need not sum to the combined median. Reductions are medians/ranges of per-pair reductions, so they need not equal the ratio of the independently reported medians.

| Workload | Proto baseline → candidate | Decoration baseline → candidate | Combined baseline → candidate | Paired combined reduction (range) |
| --- | ---: | ---: | ---: | ---: |
| Primary Overworld, 1 worker | 2224.363 → 2135.304 | 442.671 → 438.347 | 2667.810 → 2575.518 | 4.152% (3.324–5.131%) |
| Primary Overworld, 4 workers | 733.644 → 716.328 | 131.039 → 128.976 | 864.363 → 844.938 | 2.377% (0.736–2.800%) |
| Second Overworld, 1 worker | 1994.369 → 1913.935 | 233.564 → 230.780 | 2230.391 → 2143.154 | 3.865% (3.552–4.144%) |
| Second Overworld, 4 workers | 572.673 → 551.555 | 68.164 → 67.669 | 640.836 → 618.820 | 3.275% (2.561–4.469%) |
| Nether, 1 worker | 969.751 → 962.970 | 87.048 → 85.903 | 1058.483 → 1048.873 | 0.730% (-2.406–1.096%) |
| End, 1 worker | 211.710 → 211.837 | 15.689 → 16.119 | 227.650 → 228.151 | -0.175% (-1.801–1.859%) |
| Primary Overworld, non-warmed, 1 worker | 4361.328 → 4256.020 | 442.196 → 437.629 | 4804.541 → 4691.376 | 2.177% (1.743–4.552%) |

The non-warmed row retains lazy generator initialization. Resource loading remains outside the generation brackets; whole-process duration was also recorded and its non-warmed paired median improves 1.733%. Nether/End show ordinary variation, including a 0.175% End combined slowdown (about 0.5 ms per batch); no material regression is established there.

Five alternating streaming pairs after a discarded pair use four workers, radius 4, seed 12345, a three-second flight at speed 4 and 120 Hz. Median fill is 1.21 → 1.19 s with identical 81 generated chunks/504 meshed sections. Flight produces identical 121 chunks/1,704 sections. Fill update p99 is unchanged at 0.041 ms; flight p99 is 0.200 → 0.208 ms and maximum update is 0.28 → 0.23 ms. All twelve runs have zero nearby unlit frames. This short route checks responsiveness and output counts; it is not a long-travel throughput claim.

## Footprint

The twelve-entry cache field occupies 312 bytes on this platform, inline in the existing aquifer object, and adds no allocation calls or independently growing buffer. It enlarges the existing transient aquifer object, which is discarded after generation; resident and saved chunk representations are unchanged. Peak RSS medians are 732,032 → 732,088 KiB (primary one-worker) and 756,724 → 759,768 KiB (four-worker). Paired RSS ranges span increases and decreases; the four-worker median difference is about 0.4% of process RSS. The release executable text grows by 3,872 bytes (about 0.075%).

## Correctness

- Fresh authenticated vanilla 26.3 aquifer oracle: **379 fixtures, 389,804 exact lines per client/server JAR**. Covers modern/legacy factories, repeated/reversed queries, negative/wrapped coordinates, nonfinite inputs, sampler/picker traces, fluid scheduling, location/status/surface caches, bounds failures and all 18 injected sampler failure/retry cases.
- Fresh terrain-fill oracle: **114 fixtures, 200 fills, 7,950,873 exact lines per JAR**, including 61 noise-aquifer fixtures, 24 structure-density fixtures, repeated/prefilled/clipped fills, raw blocks, WG heightmaps and ordered fluid postprocessing.
- Fresh material-stage oracle: **79 fixtures, 106 surface builds, 42 generated-carving passes, 6,733,382 exact lines per JAR**, including real biome neighborhoods, aquifer traces, carving masks, top-material queries, complete sections, heightmaps and postprocessing.
- **36/36 exact baseline/candidate phase snapshots match**: three dimensions × three seeds/sites × one/four workers × before/after decoration. Each region retains 81 chunks and decorates 49 interior chunks, totaling 1,458 retained chunks per phase and 882 decorations. Seeds/sites are 12345/(0,0), -1/(-32,32), and 81985529216486895/(1874990,-1874990). Snapshots include resident palettes/words and public block reads, quart biomes, all six heightmaps, ordered postprocessing, structure summaries, binary-NBT entities/block entities, complete scheduled ticks, stage/decorated/retrogen metadata and actual non-air counts. Temporary diagnostic source was removed. A final production-only rebuild with the original package feature graph is byte-identical to the measured candidate executable (SHA256 `3b4e07bd9189270c7e0a4521bd25e156f932014d976b1c4208cde4769d5fd8ea`).

These checks establish unchanged output for the declared cases. Complete world/gameplay parity and cold physical I/O are outside this comparison.

## Reproduction and evidence

Private/generated artifacts are outside Git at `/srv/bob-data/horizon-chunk-campaign-41055168/aquifer-qualification/`: preserved binaries/sources and hashes, build logs, `pairs.py`, `matrix-raw.json`, `matrix-summary.json`, every process log, streaming runner/raw/summary, snapshot source/runner/outputs, and fresh oracle reports/inputs/outputs. Oracle runners retain the repository's corpus/comparison/authentication logic, with isolated output storage and absolute hash-map keys; the worldgen runner selects only `noise_aquifer_corpus`.

```sh
CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=/srv/bob-data/horizon-chunk-campaign-target cargo build --locked --release -p horizon-world --example chunkgen_bench -p horizon-client --no-default-features --example streaming_bench
python3 /srv/bob-data/horizon-chunk-campaign-41055168/aquifer-qualification/pairs.py
python3 /srv/bob-data/horizon-chunk-campaign-41055168/aquifer-qualification/streaming.py
```

The standard warmed generation command is `chunkgen_bench RESOURCES --seed 12345 --size 16 --threads 1 --warmup --json --no-map`; use four workers for the parallel row and `--seed -1 --center 32 -32` for the second site.

Repository quick self-check passes: `CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=/srv/bob-data/horizon-chunk-campaign-check-target python3 tools/pr_ready.py --quick --base origin/main` (4 min 23 s). Public-tree validation, workspace rustfmt, clippy with warnings denied, **1,442 client/core/world tests** and the doc-test step pass. The quick check lists affected parity harnesses; the three fresh relevant oracle suites above were executed separately. Full log: `aquifer-qualification/pr-ready.txt`.
