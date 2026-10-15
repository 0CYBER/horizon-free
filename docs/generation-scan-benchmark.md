# Generation scan gate benchmark

`Streaming::update` now reuses an exhausted generation scan until its range,
loaded membership, or generation state changes. Pending candidates keep the
original per-frame bounded scheduling. Worker generation results and proto
pruning invalidate the gate; light/mesh results do not. Public extra/forced
ranges are compared exactly. `ChunkMap` tracks structural membership and map
replacement, while value edits and existing-key replacement keep its token.
The empty-extra/forced stationary gate is constant time; comparisons scale with
those auxiliary ranges when present.

Reproduce from the repository root:

```sh
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/bob/Documents/horizon/target \
  cargo test -p horizon-client --lib --profile fast generation_scan_benchmark \
  -- --ignored --nocapture
```

The ignored test exercises the production candidate helper called by
`Streaming::update`, compared against an independent copy of the original
scan. It validates scheduled coordinates, job counts, and final membership
and in-flight state. It uses a radius-32 decorated world: 5,041 scanned square
positions, 3,409 finished resident keys and 552 outer-margin proto keys, capacity 12. Each sample runs 2,000
frames, with one warmup sample and eight measured samples; values below are
median nanoseconds per frame. The dynamic case removes the centre column every
32 frames, schedules regeneration, and accepts a saved-load completion on the
following frame.

Measured on Linux x86-64, AMD Ryzen 5 5600X, Rust 1.99.0
(`b940084d7`, 2026-09-28), optimized `fast` profile, 2026-10-14:

| Scenario | Original | Gated | Speedup | Scans per 2,000 frames | Jobs original/gated |
| --- | ---: | ---: | ---: | ---: | ---: |
| Stationary fully resident | 15,441.3 ns | 10.1 ns | 1,528.99x | 2,000 / 0 | 0 / 0 |
| Dynamic remove/load cycle | 14,794.3 ns | 1,004.8 ns | 14.72x | 2,000 / 126 | 63 / 63 |

This is a generation scheduling microbenchmark, not a full-update, FPS, worker
throughput, or startup benchmark. No prepared assets are needed: the fixture
shares an empty real `Chunk` under different resident keys and uses unit
protos because this path only reads membership. It excludes decoration execution, terrain generation, lighting,
meshing, uploads, and other per-frame scans. The stationary speedup reflects
replacing a completed scan with a small gate and should not be extrapolated to
overall game performance. Timing and assertion overhead are included in both
implementations. Memory cost is a u64 token per resident map plus a small gate
snapshot and copies of extra/forced range metadata; no resident-key cache.

Focused differential tests also cover movement, radius changes, extra and
forced areas, removal/replacement at unchanged count, entire world reset,
capacity pressure, staged generation membership, proto discard/retry, and
light/content edits that should leave the gate reusable. `ChunkMap` tests cover
structural tokens and value/light mutation separately.
