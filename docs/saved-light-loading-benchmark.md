# Saved-light loading benchmark

Saved-light reconstruction reduces complete saved-chunk loading by a median **6.179% on the normal world** and **10.981% on the flat world**, incremental to the accepted palette-loading optimization. Every measured process pair improves. The implementation changes only `save.rs::load_light` and its tests.

## Implementation and scope

Detect uniform stored nibble layers before allocating expanded resident light. Both nibbles must match: repeated bytes such as `0xF0` and `0x0F` remain nonuniform. Decode two resident light values per stored byte for remaining layers. Preserve section compaction, low-nibble-first indexing, missing-block darkness, missing-sky bottom-row inheritance, skyless behavior, section bounds, last duplicate section, invalid array handling, and loaded flags. Pinned Minecraft 26.3 `DataLayer` and `SkyLightSectionStorage.repeatFirstLayer` bytecode was inspected first. No saved-light optimization overlap was found among the 34 open PRs checked on 2026-10-22.

## Measurements

Baseline `411c5574` includes accepted palette loading and aquifer changes; the candidate differs only in saved-light reconstruction. Both loader executables were rebuilt from those sources using identical benchmark source (SHA256 `b6f5042731d7c9d5622758043f7c1e125a3f7d098c57ee0501012474c2df7325`), dependencies, resources and flags. Machine: AMD Ryzen 5 5600X, 6 cores/12 logical CPUs, 32 GiB RAM; Linux 7.2.11-zen1-1-zen; Rust 1.99.0. Release optimization, thin LTO, one codegen unit, client default features disabled; CPU boost enabled, powersave governor, ordinary desktop activity. No builds, tests or profiles overlapped timing.

Each corpus uses one discarded process pair followed by five measured pairs in alternating baseline/candidate order. Each observation is a process's median of five rounds loading the same sorted 256 full chunks. Checksumming and setup are outside timing; allocation/destruction and region reads/decompression/NBT/native decode/saved light are inside complete loading. Storage is warm. Reductions are medians/ranges of paired reductions; independently reported medians need not give the same ratio. Separate stage loops do not sum to a timing breakdown.

| Corpus | Complete load, baseline → candidate | Paired reduction (range) | Inflate/NBT, baseline → candidate | Native decode, baseline → candidate |
| --- | ---: | ---: | ---: | ---: |
| Normal vanilla 26.3, seed 12345 | 50.036 → 47.005 ms | 6.179% (5.217–6.836%) | 22.187 → 22.087 ms | 20.464 → 21.621 ms |
| Flat vanilla 26.3 | 12.030 → 10.688 ms | 10.981% (10.423–11.751%) | 4.562 → 4.549 ms | 4.076 → 4.183 ms |

Values are milliseconds per complete 256-chunk batch. The separate native-decode loop slows by a paired median 4.644% on normal and 3.185% on flat; the flat observations are variable. Its source is unchanged, and the cause of this separate-loop change was not isolated. Complete `SaveDir::load`, which includes native decode and saved light, improves in all ten measured pairs. The earlier two-pair normal pilot improved 5.894% and 6.553%; final-source qualification was repeated after the iterator syntax change. All timed compressed-record and block/biome/light hashes agree. Normal compressed records: `ff9bf9e822e334e117e3d6ba5c26a88b7698456ec6d83bdbe4355ad0af1d6f55`; block/biome/light: `661dd26eed78b051afeca11d8ba69222162b5da72e5d1572a12ce2a3b3a4f764`.

## Correctness

All 20 save unit tests pass. The new differential test compares the original loader with the candidate for every repeated packed byte, all 256 pairs of uniform sky/block nibble values, nonuniform inherited rows, missing/wrong layer types and lengths, skyless dimensions, out-of-range/invalid section Y values and duplicate sections. It compares every resident light value, uniform/full representation, block/biome storage and loaded flags.

A fresh pinned Java 26.3 oracle uses `DataLayer.get` and `SkyLightSectionStorage.repeatFirstLayer` over all 256 repeated byte patterns in both sky modes. Baseline and candidate exactly match all **50,331,648 light values** plus flags (SHA256 `608bab077edc713110f827740cb5d9baf2af9c1a251ea2458441e2d981f25920`).

The same comprehensive snapshot harness runs on the actual accepted-source baseline and final candidate: **400 normal chunks, 289 flat chunks, and two synthetic unknown-NBT/nonempty-tick cases**. All 691 outputs match byte-for-byte. It covers resident blocks/biomes/light, complete reencoded NBT, heightmaps, block entities and their saved sources/order, structures, unknown NBT, ticks, postprocessing, entities, names and loaded/arrival flags. Light representation is additionally covered by the unit test. Temporary diagnostic sources were removed.

The final source uses the fixed-size chunk iterator syntax required by Clippy. Its rebuilt executable (SHA256 `40d7031e7d3efe785815b1286af05553807bbc59f66a9275a5f89191f0ecda7e`) is the one used for final qualification; snapshot/oracle harnesses use the same production source. Earlier pilot artifacts are retained separately: section extraction normalized ELF metadata (original timed SHA256 `eb28209fd54b6deedab1ff701c291ae36b8116b2ba52a02d1ec82600b57e427f`; normalized SHA256 `78f2092b2661655fdea8d0fa2247389ccc8f48f37dd9abfbb8d8864867187297`). Those earlier artifacts are superseded by the final-source qualification.

The required repository quick self-check passes: public-tree validation, formatting, Clippy with warnings denied, **1,443 client/core/world tests**, and the doc-test step. Command: `RUST_TEST_THREADS=1 CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=/srv/bob-data/horizon-chunk-campaign-check-target python3 tools/pr_ready.py --quick --base origin/main` (11 min 15 s). Tests were serialized after an initial client-test process SIGSEGV; the failed log is retained as `pr-ready-attempt1.txt`. Relevant oracle checks were run separately; `--quick` lists other affected parity harnesses for CI.

## Reproduction and evidence

Private/generated evidence: `/srv/bob-data/horizon-chunk-campaign-41055168/saved-light/`. Preserved baseline/candidate sources and executables, source/binary hashes, open-PR catalog, pinned bytecode, build logs, `final-pairs.py`, `final-raw.json`, final qualification logs and `final-summary.json`; earlier pilot/qualification artifacts are retained there. Resources: `/srv/bob-data/horizon/.local/horizon-client/26.3`. Normal corpus: `/srv/bob-data/horizon-chunk-campaign-41055168/vanilla-normal/world/dimensions/minecraft/overworld`; flat: `/srv/bob-data/horizon/.local/locator-bar-parity/generate/world/dimensions/minecraft/overworld`.

```sh
CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=/srv/bob-data/horizon-chunk-campaign-target cargo build --locked --release -p horizon-client --no-default-features --example load_bench
load_bench RESOURCES DIMENSION_DIR --rounds 5 --limit 256 --checksum
python3 /srv/bob-data/horizon-chunk-campaign-41055168/saved-light/final-pairs.py final 5
```

These results cover the declared warm saved-load workloads. Cold physical I/O, remote chunk streams and complete gameplay parity are outside this comparison.
