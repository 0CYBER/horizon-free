# Bulk unary density evaluation benchmark

**Qualification: yes for the representative initialized Overworld workloads.** One focused bulk density arithmetic change reduces whole-terrain wall time by 13.891% with one worker and 11.191% with four workers at the primary site. A predeclared second seed/site improves 13.657% and 13.701%. These are whole-region terrain elapsed / input chunk count, not stage-only or microbenchmark results. Terrain plus decoration also exceeds 5% at both Overworld sites and worker counts. Nether/End results are smaller and are reported below.

Baseline: `e4f8154cb4115aa1ee622381d9b12387cdf515d4`. Candidate: that baseline plus `noise-aquifer-results/candidate.patch` (the scoped production change and its tests). Branch `t3/optimize-noise-aquifers`, isolated checkout `/home/bob/.t3/worktrees/horizon/t3-e477fa56` (physical `/srv/bob-data/.t3/worktrees/horizon/t3-e477fa56`). No other checkout was changed.

## Focus and feasibility

The existing initialized profile attributed 18.09% terrain self CPU to unary density arithmetic. Inspection found an operation match and function call for each buffer element. The candidate dispatches once per bulk buffer, allowing fixed-operation loops and compiler vectorization. Scalar arithmetic retains its existing dispatch; all arithmetic expressions have one shared definition. Non-finite constant operands retain the original per-element path because specialization can select a different NaN operand payload. Sampling calls, child-failure propagation, complete buffer-tail transformation, floating-point expressions and buffer allocation remain the same.

The requested reference was inspected on its oldest and newest supported source branches. Its direct palette write still maintains section counters and resolves/grows the palette. Horizon already bypasses several generic costs, while exact old-state/count and partial-write behavior still matters. A validated palette-token writer and dense palette construction were assessed as materially different designs; neither was rejected merely because an earlier small hash change failed. The profile provided a stronger focused opportunity in unary arithmetic, so no storage rewrite was attempted. See [reference mechanism and applicability](../noise-aquifer-results/reference-assessment.md) for pinned source links and details.

This shared density kernel also accelerates bulk material-density preparation. At the primary one-worker site the noise bracket improves 5.772879 → 5.229395 ms/chunk (9.414%), while surface improves 4.467371 → 3.314584 (25.805%). The 13.891% whole-terrain result must not be described as a noise-only stage improvement. No surface rules, traversal, decoration, structures, scheduling or caches were edited.

## Declared method

Workloads were fixed in `noise-aquifer-results/workloads.json` before candidate editing: 16×16 terrain chunks (256), 14×14 decorated interior (196), seed 12345 / center (0,0), all three dimensions at one and four workers. The second Overworld site uses seed -1 / center (32,-32), also at one and four workers. Each initialized process first generates and discards the same offset 9×9 region at center+(64,64), using the same seeded generator, before timing starts; measured workers are newly created. This excludes initial ring work without pre-generating measured chunks. Initialization timings remain in the logs/summary.

Each case has one discarded process pair, then five measured pairs alternating baseline/candidate order. Processes, builds and benchmarks were sequential. The cold one-worker Overworld case omits initialization and is reported separately. Wall timing is the diagnostic’s exact elapsed / 256 JSON field, never the thread-multiplied human display. Combined timing is (terrain elapsed + complete existing phased decoration elapsed) / 256, retaining identical border and stage work. Decoration duration comes from the unchanged diagnostic’s printed duration, with its original rounding.

Machine: AMD Ryzen 5 5600X, 6 cores/12 logical CPUs, 32 GiB RAM, Linux 7.2.11-zen1-1-zen, Rust 1.99.0. `fast` inherits release with LTO disabled, 16 codegen units and incremental compilation; no extra Rust flags were set. Prepared 26.3 resources: `/srv/bob-data/horizon/.local/horizon-client/26.3`. Reused Cargo target `/srv/bob-data/horizon-stage3d-target`, preserving its `fast` symlink to `/tmp/horizon-density-eval/fast-build`. Both baseline and candidate were freshly built and copied to private versioned executables. Desktop/background activity was not isolated.

## All measured medians

Values are baseline → candidate, ms per input terrain chunk; parentheses are percent reduction. Negative individual-pair reductions mean regressions.

- **overworld, 1 worker(s), seed 12345, center (0, 0), initialized:** terrain 12.270935 → 10.566346 (13.891%); combined 14.202495 → 12.541112 (11.698%). Paired terrain range 12.218% to 15.467%; candidate slower in 0/5 pairs.
- **overworld, 4 worker(s), seed 12345, center (0, 0), initialized:** terrain 3.837939 → 3.408437 (11.191%); combined 4.405165 → 3.987592 (9.479%). Paired terrain range 7.647% to 13.352%; candidate slower in 0/5 pairs.
- **the_nether, 1 worker(s), seed 12345, center (0, 0), initialized:** terrain 5.000868 → 4.843412 (3.149%); combined 5.381649 → 5.215208 (3.093%). Paired terrain range -3.917% to 6.809%; candidate slower in 2/5 pairs.
- **the_nether, 4 worker(s), seed 12345, center (0, 0), initialized:** terrain 1.395740 → 1.367372 (2.033%); combined 1.497615 → 1.473427 (1.615%). Paired terrain range 0.323% to 17.443%; candidate slower in 0/5 pairs.
- **the_end, 1 worker(s), seed 12345, center (0, 0), initialized:** terrain 1.087874 → 1.054903 (3.031%); combined 1.161584 → 1.128076 (2.885%). Paired terrain range -0.144% to 14.336%; candidate slower in 1/5 pairs.
- **the_end, 4 worker(s), seed 12345, center (0, 0), initialized:** terrain 0.275263 → 0.260434 (5.387%); combined 0.302814 → 0.288325 (4.785%). Paired terrain range 4.468% to 6.365%; candidate slower in 0/5 pairs.
- **overworld, 1 worker(s), seed -1, center (32, -32), initialized:** terrain 11.333986 → 9.786101 (13.657%); combined 12.356760 → 10.854422 (12.158%). Paired terrain range 7.854% to 14.172%; candidate slower in 0/5 pairs.
- **overworld, 4 worker(s), seed -1, center (32, -32), initialized:** terrain 3.058178 → 2.639180 (13.701%); combined 3.357631 → 2.938479 (12.484%). Paired terrain range 10.767% to 14.651%; candidate slower in 0/5 pairs.
- **overworld, 1 worker(s), seed 12345, center (0, 0), cold:** terrain 23.049481 → 21.473030 (6.839%); combined 24.984204 → 23.443390 (6.167%). Paired terrain range 6.458% to 7.944%; candidate slower in 0/5 pairs.

Nether one-worker repeats 1 and 3 regress 0.555% and 3.917%; End one-worker repeat 1 regresses 0.144%. Nether four-worker and End one-worker have large favorable outliers (paired improvements up to 17.443% and 14.336%), so their small median gains should not be generalized. All Overworld initialized pairs exceed 5%; the smallest is 7.647% at the primary four-worker site. No case was removed or selected after seeing timings. Cold results include startup and are not used to establish the recurring improvement.

Primary terrain process CPU medians: 12.1 → 10.5 ms/input chunk at one worker, 15.2 → 13.5 at four workers. This existing `/proc` accounting has 100 Hz quantization and a rounded display. Aggregate stage worker elapsed is not an exclusive CPU share. Primary peak process RSS: 786588 → 786608 KiB (one worker), 834016 → 833440 KiB (four workers), effectively unchanged. All workload CPU/RSS medians are in `summary.json`; process CPU additionally includes initialization/other phases where explicitly named. No new heap buffer is introduced. Executable `.text` grows 2860 bytes, 5164975 → 5167835 bytes. Compiler output shows packed float instructions in the specialized bulk method; the original bulk method had none.

An initial variant additionally forced the shared scalar helper inline. Its excluded five-pair primary result was 12.240292 → 10.644734 ms/chunk (13.035%). This broader variant was not used for qualification; the narrowed bulk-only candidate independently reproduced the benefit. The early hypothesis that scalar inlining explained the surface gain was disproved by the narrowed result and inspection of material-density bulk preparation. Initial raw data/patch/hashes are retained under `noise-aquifer-results/initial-shared-inline/`.

## Correctness

- 191 final optimized unit/integration tests pass: 86 core library, 91 world library, 6 context/world-field integration tests, and 8 integration tests covering material stages, carving, ticks, retrogen biomes and biome regions. Storage/palette randomized editing, packing, invalid-write atomicity, density caches/errors and interpolation tests are included.
- New bit-exact tests cover every unary operation, signed zeros, subnormals, infinities, NaN payloads, 4096 deterministic random float patterns, multiple buffer lengths and varied constants. A new child-error test verifies partial input writes survive without applying the operation and verifies complete oversized-buffer tails on success. An initial non-finite-operand NaN payload failure was fixed by retaining original dispatch for those constants; the strict assertion was retained.
- Fresh candidate and Java client/server density runs each match **15,524,627 output lines**. The existing input corpus was authenticated against its recorded checksum and unchanged corpus/oracle source; both Java sides were freshly compiled/run against independently checksum-verified pinned JARs.
- Fresh scoped terrain-fill parity matches **7,950,873 lines on each Java side**: 114 fixtures, 200 fills, 61 noise-aquifer fixtures and 24 structure-density fixtures. This covers actual noise fill, prefilled sections, partial vertical volumes, repeated fills, both seeded factories, bundled routers/aquifers, raw section states, WG heightmaps and ordered postprocessing. The repository harness was changed only in memory to use `fast` build/profile paths and isolated private output storage; the adaptation script and its hashes are saved.
- Full baseline/candidate chunk differential: 12 cases (three dimensions × two seeds/sites × one/four workers), each with 81 terrain chunks and 49 interior decorations. All 81 retained chunks, including borders, are compared both after terrain and after decoration: **24/24 exact byte comparisons pass**, covering 1,317,207,694 bytes per revision. Snapshots include raw and public block reads, section palettes/storage, all quart biomes, block entities and entities (binary NBT preserving numeric bits and order), complete saved block/fluid ticks including delay/priority, all six heightmaps with presence/raw words, ordered postprocessing and actual non-air counts. Chunk status, decoration flag, retrogen metadata and structure identities/bounds are included.
- Snapshot builds use the same temporary read-only accessor for actual non-air counts on both revisions. It is removed from production source. The final production-only benchmark rebuild has byte-identical `.text` and `.rodata` to the measured candidate; the twelve changed read-only relocation-data words are source-location line numbers decreasing by four after removing the accessor. Binary/codegen hashes are saved.
- `git diff --check` passes. The repository PR self-check (`pr_ready.py --quick`) also passes: public-tree validation, 236 Python tests, workspace rustfmt, workspace/all-target clippy with warnings denied, and 94 full core-crate tests. Java parity was run separately as detailed above. The self-check size warning counts generated raw measurement/validation artifacts; the production/test Rust diff is 227 changed lines in two files. See `noise-aquifer-results/pr-ready.md`.

Validation is scoped to these workloads and corpora, not every seed, biome or future compiler. Whole-chunk parity is a complete comparison of the listed generated fields against the baseline; it is not a claim that every untested generation behavior matches Java. Full-chunk Java gameplay, lighting, meshing, saving and streaming were not benchmarked. No required affected metadata was omitted from the baseline/candidate differential.

## Evidence and reproduction

Public evidence is under `noise-aquifer-results/`: `workloads.json`, `environment.json`, `binary-hashes.txt`, `parity-binary-hashes.txt`, `final-codegen-check.json`, `gate-raw.json`, `matrix-raw.json`, `summary.json`, `density-parity.json`, `terrain-parity.json`, `chunk-parity.json`, test logs, diagnostic sources and runners. Raw measurements include discarded warmups, exact commands and every measured pair; summaries exclude only the declared warmups. Private executables, density outputs and full chunk snapshots are under `/srv/bob-data/horizon-noise-aquifer-e477fa56/`; noise-fill fixture data is ignored under `noise-aquifer-results/private-parity/`. No private fixtures, chunk dumps, game resources or Java binaries are committed.

For fresh baseline/candidate performance builds in this isolated checkout, use the saved `chunkgen_profile.rs` as `crates/horizon-world/examples/chunkgen_profile.rs`, build each source variant with the command below, and copy its executable to the matching `baseline` or `candidate` path expected by `bench.py`. Baseline is the pinned commit; the candidate patch is saved independently. The measured candidate also had the unused diagnostic accessor available; it emits no benchmark code. Remove copied example/accessor support afterward.

```sh
CARGO_TARGET_DIR=/srv/bob-data/horizon-stage3d-target cargo build --profile fast -p horizon-world --example chunkgen_profile
python3 noise-aquifer-results/bench.py gate
python3 noise-aquifer-results/bench.py matrix
python3 noise-aquifer-results/summarize.py
```

For differential snapshots, copy `chunkgen_snapshot.rs` into the examples directory and apply `snapshot-accessor.patch`, then run `python3 noise-aquifer-results/build_snapshots.py` (it saves/restores the candidate arithmetic source and builds both variants). Run `validate_chunks.py`, then remove the temporary accessor and copied examples. The density runner expects a fast `horizon-density-parity` binary copied to the private executable directory; `build_snapshots.py` also produces it. Run `validate_density.py`. Run the scoped terrain oracle with `CARGO_TARGET_DIR=/srv/bob-data/horizon-stage3d-target python3 noise-aquifer-results/validate_terrain.py`; its fixtures stay in ignored private storage. The paths are intentionally explicit and can be adjusted together on another machine.

Final unit/integration commands:

```sh
CARGO_TARGET_DIR=/srv/bob-data/horizon-stage3d-target cargo test --profile fast -p horizon-core --lib --test density_world_fields
CARGO_TARGET_DIR=/srv/bob-data/horizon-stage3d-target cargo test --profile fast -p horizon-world --lib --test material_stage --test carving_stage --test tick_data --test retrogen_biomes --test biome_region
git diff --check
```

## Advice for the surface task

Reprofile with this change present before evaluating a separate surface optimization. Shared bulk density preparation already accounts for about 1.15 ms/chunk of the primary surface-bracket saving; do not count it again as a later task’s improvement. Compiled rule evaluation and remaining column traversal are unchanged. No task in another area was started.
