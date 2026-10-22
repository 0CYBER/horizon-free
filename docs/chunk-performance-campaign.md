# Chunk generation and loading performance campaign

Profiling baseline: `41055168afcd8687c7efe525545de896f17f4478` (2026-10-22). No production optimization is included in this profiling step. Measurements identify work to investigate; they do not establish a candidate speedup or complete Minecraft world parity.

## Reproduction and artifacts

Machine: AMD Ryzen 5 5600X, 6 cores/12 logical CPUs, 32 GiB RAM, Linux 7.2.11-zen1-1-zen x86_64; Rust/Cargo 1.99.0, OpenJDK 25.0.4.1. CPU boost remained enabled, governor `powersave`; normal desktop background activity was not isolated. Builds, timed workloads and sampling runs were sequential. Release uses `opt-level=3`, thin LTO, one codegen unit; client benchmarks disable audio/controllers and need no window/GPU. Do not compare these numbers directly with earlier `fast` profile reports.

Private/generated evidence is outside Git at `/srv/bob-data/horizon-chunk-campaign-41055168/`: `baseline-raw.json`, `baseline-summary.json`, `run_baseline.py`, individual logs, preserved executables in `baseline/`, `binary-sha256.txt`, `fixture-sha256.txt`, the `*-profile.er` sampling experiments, and `*-functions.txt` tables. The matrix binaries are preserved separately from subsequent `*-profile` binaries. The latter add a structure timing field or optional worker execution for profiling; production sources are identical. Prepared resources: `/srv/bob-data/horizon/.local/horizon-client/26.3`. Pinned jars and source artifacts: `/srv/bob-data/horizon/.local/minecraft/26.3`.

```sh
CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=/srv/bob-data/horizon-chunk-campaign-target cargo build --locked --release -p horizon-world --example chunkgen_bench -p horizon-client --no-default-features --example chunk_bench --example streaming_bench --example save_bench --example load_bench
```

`chunkgen_bench` now accepts `--warmup --json --no-map`. Warmup generates/discards an offset 9×9 region at center+(64,64), with the same seeded generator and a separate worker, before creating fresh measured workers. This removes lazy ring initialization from the steady workload without generating target chunks. Resource loading, generator construction, warmup, diagnostic output and map rendering remain outside the measured generation brackets. `BENCH` JSON reports exact elapsed seconds, process CPU accounting and cumulative stage wall brackets. Worker stage brackets are not exclusive CPU shares; elapsed/count has no thread multiplier. Non-warmed runs should be measured separately when changing startup behavior.

`load_bench RESOURCES DIMENSION_DIR --rounds 5 --limit 256 --checksum` is read-only. It selects full chunks in sorted region/coordinate order, primes region handles and filesystem cache, then separately times `SaveDir::load` (region read, decompression, NBT, resident decode and saved light), decompression/NBT of retained compressed records, and native decode of retained tags. Allocation and destruction are inside each timed loop. Setup and checksums are outside. Stage times come from separate loops and must not be summed as an exact breakdown. It emits compressed-record SHA256 provenance and an optional complete blocks/biomes/light checksum; this checksum does not cover all saved metadata.

## Baseline observations

The primary workload uses seed 12345, center (0,0), size 16: 256 proto chunks and 196 interior decorations. One-worker/four-worker results have five processes each; Nether/End have three each. Values are medians.

| Workload | Proto wall | Decoration wall | Combined ms/input chunk |
| --- | ---: | ---: | ---: |
| Overworld, 1 worker | 2.224094 s | 0.440144 s | 10.407179 |
| Overworld, 4 workers | 0.732982 s | 0.131088 s | 3.375274 |
| Nether, 1 worker | 0.981417 s | 0.084589 s | 4.164085 |
| End, 1 worker | 0.213254 s | 0.015784 s | 0.894681 |

The one-worker Overworld brackets are approximately noise 4.20, surface 2.55, biomes 0.40 and carvers 0.34 ms/input chunk. A separate sampled size-32 run reports structures 0.395 ms/input chunk; its larger site mix differs from size 16.

Resident `chunk_bench --seed 12345 --size 4`, three processes: finish 0.4 ms/chunk (rounded display), light median 1.43 ms/column, mesh 109.5 µs/section (each process reports its best of three mesh sweeps), 29.8 KiB resident storage/chunk. Before qualifying a finish/light/mesh change, emit exact timings on both revisions; existing rounded/best-of-three output is exploratory evidence only.

`streaming_bench --threads 4 --radius 4 --seed 12345 --fly 4 --seconds 3 --hz 120`, three processes: fill median 1.21 s, 81 chunks generated and 504 sections meshed; median reported update p99 0.041 ms. Flight generates 121 chunks, with median update p99 0.207 ms and no sampled frame with an unlit column within two chunks of the player. This short fixed-duration flight is a responsiveness/throughput check, not a complete long-travel stress test.

Saved loads use 256 full chunks, three independent processes, five measured rounds/process; table entries are the median of process medians for the whole 256-chunk batch.

| Corpus | Complete load | Inflate + NBT | Native decode |
| --- | ---: | ---: | ---: |
| Normal vanilla 26.3, seed 12345 | 55.740 ms | 22.068 ms | 27.211 ms |
| Flat vanilla 26.3 locator fixture | 12.733 ms | 4.505 ms | 4.884 ms |

The normal fixture was generated by the pinned local vanilla server, `level-type=minecraft:normal`, seed 12345, forced block bounds -128,-128 through 127,127, followed by `save-all flush` and clean stop. It contains 400 full chunks; the sorted limit selects 256. Files remain at `vanilla-normal/world/dimensions/minecraft/overworld` under the artifact root. Flat source is `/srv/bob-data/horizon/.local/locator-bar-parity/generate/world/dimensions/minecraft/overworld`. Normal compressed-record SHA256: `ff9bf9e822e334e117e3d6ba5c26a88b7698456ec6d83bdbe4355ad0af1d6f55`; blocks/biomes/light: `661dd26eed78b051afeca11d8ba69222162b5da72e5d1572a12ce2a3b3a4f764`. All three processes agree, and all 256 selected chunks have saved light. Flat checksums are in the raw logs. These are warm storage measurements; cold physical I/O and a remote multiplayer chunk stream are not measured.

`save_bench` also read the 400 normal chunks and checked written compressed records byte-for-byte: median encode 139 ms, compression 141 ms, parallel SaveDir encode/compress/write 54 ms, five rounds. This checks existing save output consistency, not vanilla read/write parity.

## CPU samples and ranked investigations

`perf`, Valgrind and samply are unavailable; GNU gprofng 2.47 is installed. Collection uses `gprofng collect app -p hi -o OUTPUT.er BINARY ARGS`, nominally 997 µs samples. On this machine the collector warns that the initial thread's timer is cleared (`997 -> 0`). Initial-thread samples are therefore unusable. The measured terrain worker (thread 4) records 9.204 sampled CPU seconds against 9.22 process CPU seconds in its generation bracket; decoration worker threads record 3.098 seconds against 3.126 wall seconds. Their worker-only tables are retained as `terrain-worker-functions.txt` and `decoration-worker-functions.txt`. Startup/main-thread percentages must not be inferred from these samples. Streaming worker samples independently reproduce the same generation hotspots.

The initial loader experiment captured only 0.108 sampled seconds against 10.899 process CPU seconds and is rejected. Repeating the identical code with optional `load_bench --profile-worker --rounds 100` captures 11.323 sampled seconds against 11.347 process CPU seconds; its table is `load-worker-profile-functions.txt`. In this mixed complete-load/NBT/native-decode workload, `PalettedContainer::from_packed` is 16.12% exclusive / 16.62% inclusive, zlib `inflate_fast_help_avx2` 15.71% exclusive, saved `load_light` 5.80% exclusive / 7.38% inclusive, and NBT `Reader::payload` 4.08% exclusive / 15.06% inclusive. Native `decode_column` is 46.97% inclusive. Formatting (`format_inner`) is 6.52% inclusive, including property-map Debug formatting at 4.79%; these nested shares cannot be added. These are shares of all three benchmark loops, not just the complete-load loop. Palette reconstruction/validation is the strongest loading lead; replacing the formatted per-chunk state-cache key is a smaller plausible lead requiring the full-load 5% gate.

For the size-32 Overworld terrain worker, ranked opportunities outside the excluded PRs are:

1. Aquifer `compute_substance`: 13.00% exclusive / 15.83% inclusive CPU. Investigate repeated nearest-cell/grid work while retaining exact distance ties, arithmetic, random draws and fluid scheduling.
2. Material `CompiledRule::apply`: 8.52% exclusive / 14.58% inclusive. Check the already-merged surface work before changing anything; inspect fresh evidence for remaining dispatch/data access, preserving condition order and random consumption.
3. `BlockSection::set` 6.09% and `get` 2.79%; decoration additionally spends 9.95% in `get`. Investigate construction/access patterns rather than changing vanilla palette semantics. Shared storage changes need broad callers and palette-width boundary validation.
4. Beardifier `sample_unchecked`: 4.78% exclusive. Recent bulk-density work is already in baseline; only a distinct measured bottleneck should proceed.
5. Decoration sculk `spread_all`: 8.82% exclusive / 22.85% inclusive of decoration-worker CPU at this larger site. Strongly site-dependent; require a matched sculk corpus before claiming a whole-workload win.

Open/overlapping work excluded from optimization: #1208 Perlin floor/int conversion and float gradients; #1206 Perlin volume/climate lookup; #1207 feature-region grid, ore column/visited handling, lazy biome temperature; #1201 player proximity/ticket/requeue grids; #1192 worker priority; #1191 content/assets loading. Existing merged sampler reuse, neighbor generation, carver cache, surface, packed lighting and bulk density changes are part of this baseline. These exclusions cover the semantic mechanism, not merely the file paths.

## Candidate acceptance protocol

For each independent candidate, preserve identical benchmark sources, resource/corpus hashes, build flags and baseline/candidate executables. Do not compare against an open PR's branch. Run no builds, parity jobs or profilers concurrently with timed qualification. Use one discarded baseline/candidate process pair, then at least five measured pairs, alternating A/B and B/A order. Keep every observation; if external contention invalidates a batch, retain it and restart the entire batch with an explanation.

Primary generation command: `chunkgen_bench RESOURCES --seed 12345 --size 16 --threads 1 --warmup --json --no-map`, repeated at four workers. Add seed -1 at center (32,-32), Nether and End, with five pairs for any affected claim. Report proto, decoration and complete proto+decoration wall time; input and decorated chunk counts must match. Retain a non-warmed check for startup regressions and streaming fill/flight for player-visible effects.

Primary loading command: `load_bench RESOURCES NORMAL_DIMENSION --rounds 5 --limit 256 --checksum`, repeated with the flat fixture. Each process's median complete `load_s` is one observation; internal rounds are not independent processes. Report native decode and inflate/NBT alongside complete load. Use the same command without profiler-worker mode on both revisions for qualification.

Open a PR only after the affected complete workload improves by at least 5% in the paired repeated measurements, the improvement is stable across the measured pairs, and correctness gates pass. Report median reduction and range of paired reductions; add pairs if noise makes the 5% threshold ambiguous. A 5% isolated-stage win without a 5% complete affected-workload win does not qualify. Check related workloads for material regression. Explain corpus-specific results instead of claiming universal speedups.

Correctness gates must include exact baseline/candidate generated blocks, quart biomes, heightmaps, postprocessing, structures, block entities and metadata for affected cases, preserving RNG and iteration behavior, plus the pinned vanilla oracle. Use existing `tools/test_chunk_generation_parity.py` (full world stage comparisons), relevant noise/density/aquifer/material/feature/palette harnesses, and `tools/test_light_parity.py` if lighting changes. Cover negative/far coordinates, several seeds, all affected dimensions and both worker counts. `tools/parity.py --repeat 3 ...` checks repeated oracle/probe agreement; reused Java oracle output is suitable for Rust-only comparisons, with a fresh relevant oracle run before final qualification.

Loading changes additionally require existing save unit tests and comprehensive baseline/candidate semantic output checks, including heightmaps, block entities, ticks, structures and preserved unknown NBT. The benchmark's block/biome/light checksum is only one gate. Use `tools/test_save_roundtrip_parity.py` for the full vanilla → Horizon → vanilla path when relevant. Prepared Java 25/pinned jars/resources make these checks feasible here; full roundtrip requires the repository's virtual-display/runtime setup and has not been run in this profiling step. No complete world-parity pass is claimed by this report. Run `python3 tools/pr_ready.py --base origin/main` and repository-required checks before any PR.

## Agent 2: saved palette validation

Candidate: list palettes in `PalettedContainer::from_packed` keep the saved words directly, so avoid allocating/filling the discarded unpacked-index vector. Reuse index validation, scanning words and their occupied slots in entry order; a palette filling the bit capacity cannot contain an invalid index. Global reencoding remains unchanged. The first invalid index, exact storage-length errors, unused high padding, palette order and resident words are preserved. Pinned 26.3 `PalettedContainer.unpack` bytecode was inspected first: matching list configurations retain `SimpleBitStorage` directly; global configurations reencode. No palette-related open PR overlap was found on 2026-10-22.

Evidence: `/srv/bob-data/horizon-chunk-campaign-41055168/palette/`, `palette-pairs.py`, `palette-pilot.log`, `palette-qualification.log`, `palette-candidate.patch`, `palette-java-bytecode.txt`, and `palette-*.log`. Binary hashes and commands are recorded in `palette/binaries.json` and `*-raw.json`. The preserved baseline `load_bench-profile` is used without worker/profiler flags because its source exactly matches the candidate benchmark source, including the optional worker branch. Identical release flags, resources and compressed records were used. No builds, tests or profilers overlapped qualification.

Three pilot pairs showed 9.77–10.28% complete normal-load reductions after a discarded pair. Qualification used five alternating process pairs per corpus, each after a discarded pair, with five measured rounds per process. Values below are medians of process medians for each 256-chunk batch; reductions are medians/ranges of the paired reductions.

| Corpus / workload | Baseline | Candidate | Paired reduction (range) |
| --- | ---: | ---: | ---: |
| Normal complete load | 55.459 ms | 49.770 ms | 10.257% (9.656–10.573%) |
| Normal native decode | 26.263 ms | 19.902 ms | 24.188% (22.323–25.987%) |
| Normal inflate + NBT | 22.260 ms | 22.086 ms | 0.782% (-1.174–1.680%) |
| Flat complete load | 12.663 ms | 12.027 ms | 5.023% (2.609–5.617%) |
| Flat native decode | 5.170 ms | 4.220 ms | 13.908% (5.850–28.399%) |
| Flat inflate + NBT | 4.537 ms | 4.548 ms | -0.159% (-1.852–0.563%) |

Normal clears the complete-workload 5% gate in every measured pair. Flat shows no observed regression, but its variation does not support a stable at-least-5% claim. This is a warm saved-load improvement on these fixtures, not a generation or cold-I/O claim.

Correctness: all timed baseline/candidate compressed-record and block/biome/light hashes match. Core palette tests pass (7 passed; 2 existing ignored). A differential reference test covers all saved widths through 13, single/list/global modes, duplicate values, full-capacity palettes, absent/wrong storage, dirty word/final padding, and first-invalid-index errors; successful cases compare strategy, bits, palette and words exactly. A fresh pinned Java palette oracle passes 266 scenarios and 861,864 edits with identical hashes. Existing `save::tests` pass (19/19), including malformed packed palettes, stored light, block-entity roundtrips, file upgrades and level-file roundtrip.

The temporary `load_snapshot.rs` harness and true baseline/candidate binaries are archived under `palette/`; the harness was removed from the working tree. It hashes resident palette/word representations and light values, canonical complete reencoded NBT (including heightmaps/block entities), saved root NBT/block-entity sources/order/ticks, and loaded flags, names, postprocessing, entities and structure references. Every full chunk was checked: 400 normal and 289 flat, plus a synthetic nested unknown NBT key and nonempty block/fluid ticks for each corpus. All 691 per-case baseline/candidate outputs match byte-for-byte. `palette/correctness.json` records hashes; `palette/sources.json` authenticates both core sources and the harness. Exact candidate source was restored after the baseline comparison.

Both baseline and candidate also ran `save_parity` against the pinned vanilla-generated worlds: 324 interior normal chunks have zero block, biome, heightmap or other metadata differences, with the same preexisting 7/63,700,992 recomputed light-value differences in both revisions. All checks on 225 interior flat chunks are identical to vanilla, including all 44,236,800 light values. This establishes no added discrepancy in these comparisons; it is not a clean normal-world lighting parity pass. The full server/client `test_save_roundtrip_parity.py` simulation was not run; the candidate changes only loading/validation, and the read-only full-output comparison covers unchanged save output. No generation parity claim or PR is included in this agent's result.


## Round 3: aquifer neighborhood pilot — rejected

Inspected the pinned server 26.3 `Aquifer.NoiseBasedAquifer.computeSubstance` bytecode and current open PRs; no aquifer optimization overlap was found. Tested a single-current-grid cache of the 12 decoded candidate positions/indices, populated in vanilla cell order only after successful checked accesses, plus explicit four-slot nearest insertion preserving later-cell distance ties. The candidate touched only `worldgen/aquifer/noise_based.rs`; no noise evaluation or terrain traversal was changed.

Evidence is retained in `/srv/bob-data/horizon-chunk-campaign-41055168/aquifer/`: baseline source, candidate patch, preserved candidate executable, binary hashes, build log, pilot script, all process logs, `pilot-raw.json` and `summary.json`. Pinned Java bytecode is in `aquifer-java-bytecode.txt` at the parent artifact root. The benchmark source SHA256 is `f404feb7ec526f12735d11b15af36254fd585f39b975cdbdc71905d87f10f8cc`, identical to the preserved `chunkgen_bench-profile` baseline source. Both executables use the established release flags and resources. No builds, tests or profilers ran during timing.

One discarded process pair followed by three alternating pairs used the primary seed-12345, center-(0,0), size-16, one-worker warmed command. Every process generated 256 proto chunks and decorated 196 chunks. Complete proto+decoration paired reductions were **3.864%, 4.326%, 3.828%** (median **3.864%**); baseline/candidate complete-time medians were **2.678451 s / 2.568094 s**. This does not clear the 5% complete-workload gate. The pilot had already completed when the parent requested early cancellation. The production candidate was reverted; no qualification, four-worker/second-site matrix, parity tests, generated snapshots or PR were pursued. No correctness claim is made for the rejected candidate. This result applies to this mechanism and corpus, not an upper bound on all aquifer optimization.

Next lead: remaining material-rule dispatch/data access (8.52% exclusive terrain-worker CPU), after checking the already-merged surface changes. Preserve rule evaluation order and random consumption; reject early if its complete-workload pilot remains below 5%.


## Round 4: remaining material-rule dispatch — early no-go

Inspected merged #801 (`Specialize constant surface material-rule branches`), `docs/surface-generation-benchmark.md`, the current compiled rule/condition/density implementation, and the pinned 26.3 material resources and Java `ConditionRule`, `SequenceRule`, `OreVeinRule`, and `BlockRule` bytecode. The open-PR list was checked on 2026-10-22; no additional material-rule dispatch PR was found, and the campaign's excluded mechanisms were left untouched. Inspection evidence is retained under `/srv/bob-data/horizon-chunk-campaign-41055168/surface-feasibility/`: `pinned-material-bytecode.txt` and `inspection.json` (source commit, pinned server-game SHA256, and static expanded rule counts).

The existing implementation already removes constant predicates and singleton/empty sequences after compiling all children, shares eager density roots, and uses unit-step material-volume indexing. The 8.52% exclusive terrain-worker sample share for `CompiledRule::apply` includes ore-vein execution and inlined work, so it cannot be treated as removable dispatch overhead. Vanilla's top-level Overworld sequence includes both ore-vein rules before the surface predicate and underground rules. Condition/block fusion or flattening nested sequences would remove only part of this work; a broad replacement evaluator would be a speculative refactor without evidence for the required complete-workload gain. The raw expanded Overworld resource tree contains 53 sequences, 149 conditions, 110 blocks, two ore-vein rules and two band rules; these static counts precede constant specialization and are not runtime frequencies.

Decision: **early no-go**, accepted by the parent before implementation. No production source or tests were changed, no build/timing/parity jobs were run, and no candidate or PR was created. This is a feasibility rejection, not a measured performance result or an upper bound on future material optimizations. The rejected aquifer executable in the shared target directory was not used. Earlier campaign evidence and the uncommitted Round 3 report were preserved.


## Round 5: section access and construction — early no-go

Inspected `BlockSection` get/set/construction, `PackedStorage` addressing, the existing ignored section microbenchmark and differential tests, and merged commits `939eabe6`, `56a251eb` and `5c55e7d8`. Checked the open-PR list on 2026-10-22; no section-storage optimization overlap was found. Pinned 26.3 `LevelChunkSection`, `PalettedContainer` and `SimpleBitStorage` bytecode was inspected before considering changes. Evidence is retained in `/srv/bob-data/horizon-chunk-campaign-41055168/section-feasibility/`: `pinned-section-bytecode.txt`, `baseline-access-assembly.txt` and `inspection.json` (source/pinned-jar/baseline hashes and the bounded open-PR catalog).

The current implementation already reads the old packed index directly, resolves the incoming state with one multiplicative-hash map probe, reuses freed palette slots, and skips repeated storage validation after the section checks. The ignored microbenchmark compares against older pre-optimization implementations; its historical speedups are already in the campaign baseline and cannot qualify a new candidate. Uniform construction still allocates palette/count/map storage, but construction is not identified as a substantial exclusive hotspot in the retained profile.

The preserved baseline executable confirms runtime division in packed-index addressing; pinned Java uses reciprocal multiplication in `SimpleBitStorage.cellIndex`. This is a possible future storage investigation, but the division is only part of the sampled access cost, alongside palette/word loads, incoming-state lookup and count/free-slot handling. Terrain samples assign 6.09% exclusive CPU to `set` and 2.79% to `get`, while the separate decoration sample assigns 9.95% to `get`; these site-specific shares are not a measured removable fraction or a complete-workload speedup prediction. There is no evidence here that a small addressing or construction change can remove enough complete proto-plus-decoration work to meet the 5% gate. A dense representation or bulk terrain writer would need broader semantic and workload evidence and was not attempted.

Decision: **early no-go**, accepted by the parent before implementation. No production source or tests were changed, no build/timing/parity jobs were run, and no candidate or PR was created. No new correctness or performance result is claimed. The rejected aquifer executable in the shared target directory was not used; assembly inspection used the preserved genuine baseline. Earlier campaign evidence and uncommitted Round 3/4 reports were preserved.


## Round 6: saved block-state cache keys — rejected

Inspected the loader's formatted per-chunk state-cache keys, `BlockState::from_nbt`, registry lookup/default fallback, and pinned 26.3 `StateHolder.codec`/`BlockState` bytecode. Checked the open-PR list on 2026-10-22; no saved-state-cache optimization overlap was found. The candidate derived `Hash` for `BlockState` and replaced the formatted string key with the parsed structural state. Invalid parse results returned air directly; the existing registry/default-property resolution was retained. No palette or lighting code changed.

Evidence is retained under `/srv/bob-data/horizon-chunk-campaign-41055168/state-key/`: candidate patch and sources, preserved baseline source copies, pinned bytecode, bounded open-PR catalog, build log, paired runner, raw process observations/logs, executable hashes and summary. The baseline was the preserved **accepted palette candidate**, so these results are incremental to commit `5d82b4c9`, with none of its earlier gain counted again. Both binaries used identical `load_bench` source (SHA256 `b6f5042731d7c9d5622758043f7c1e125a3f7d098c57ee0501012474c2df7325`), release/thin-LTO/one-codegen-unit flags, resources and normal corpus. No builds, tests or profilers overlapped timing.

One discarded process pair followed by two alternating measured pairs used `--rounds 5 --limit 256 --checksum`. Complete normal-load paired reductions were **3.257% and 2.412%** (median **2.835%**); baseline/candidate process-median medians were **49.725 / 48.316 ms** per 256 chunks. Native decode improved **5.917%** (20.266 / 19.067 ms); inflate/NBT varied by **-0.844%**. Compressed-record and block/biome/light checksums matched in every process. Those checksums are limited benchmark checks, not comprehensive semantic or vanilla parity evidence.

Decision: **early no-go** at the incremental 5% complete-load gate. Both production files were restored exactly to their pre-candidate contents; only this report remains. No flat/long qualification, comprehensive snapshot comparison, fresh parity suite, tests, commit or PR were pursued for this rejected candidate. The shared Cargo target contains the rejected executable; future work must use the preserved accepted binary or rebuild restored sources.
