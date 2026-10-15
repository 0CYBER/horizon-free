# Animated sprite mip cache

`animate_sprites` retains full mip chains for previously displayed source frames.
The renderer owns an immutable snapshot of animations, size and mip settings;
resource reload replaces that snapshot and drops its cache. Mipmap options only
change the sampled view, so the full chains stay valid. Frame selection, skipped
unchanged frames, upload order and mip bytes are preserved.

Admission is lazy, with an 8 MiB limit on retained `Vec` capacities and a separate
4,096-entry limit on map metadata. The existing fixed integer hasher avoids
consuming random seeds for cache bookkeeping. An admitted frame stays until resource reload;
a frame that does not fit uses the original generation path. There is no eviction
churn. These limits apply to the additional cache, excluding the existing source
frames and the original generator's temporary allocations. Map/allocator overhead
is additional and bounded by the entry limit.

## Reproduce

Use prepared 26.3 resources at `.local/horizon-client/26.3`. All block/item
textures are loaded, including static layers that set the shared array size
(the 64px straw-bed texture). The Python driver
compiles the production `gfx.rs` functions and actual sprite/mipmap loaders into
a private Cargo workspace under `.local/sprite-mips-bench`; it does not build the
rest of the client or expose a benchmark API in the renderer. `--ref` selects the
baseline's actual gfx code; the sprite/mipmap loaders are unchanged by this
optimization. Both use release optimization, LTO off, 16 codegen units.

Build first, then release the lock before the paired measurement session. Keep
all measurement pairs in one lock acquisition so other heavy work cannot overlap:

```sh
flock /tmp/horizon-optimizations-20261015-heavy.lock bash -c '
  set -e
  export CARGO_BUILD_JOBS=2
  python3 tools/bench_sprite_mips.py --ref 89b58991bbc02eb79b6965fc4ca56357fe2297d9 --label baseline --build-only
  python3 tools/bench_sprite_mips.py --label candidate --build-only
'
flock /tmp/horizon-optimizations-20261015-heavy.lock bash -c '
  set -e
  export CARGO_BUILD_JOBS=2
  for round in 1 2 3; do
    variants="baseline candidate"
    if [ "$round" = 2 ]; then variants="candidate baseline"; fi
    for variant in $variants; do
      python3 -c "import resource, subprocess, sys; subprocess.run(sys.argv[1:], check=True); print(\"max_rss_kib=\" + str(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss), file=sys.stderr)" \
        .local/sprite-mips-bench/target/release/sprite-mips-$variant \
        .local/horizon-client/26.3 .local/sprite-mips-bench/$variant/names.txt
    done
  done
'
```

Each process reports resource loading, its first frame (including one-time mip
tables), nine warmed-code samples of fresh-state setup/first upload/teardown,
and setup plus the first 4,096 ticks. It then samples 1,024 warmed animation ticks
and 3,072 rendered frames at three renders per tick, nine times each. Another
1,024 ticks report CPU latency p50/p95/max. The warmed state is dropped before
the unique workloads, keeping one live renderer cache at a time. A black-box
upload callback consumes each slice without GPU calls. The unchanged initial
atlas generation, GPU staging, draw calls and presentation are outside this CPU
measurement; these numbers do not establish an FPS improvement.

The first-seen workloads request each distinct source frame found over 4,096
ticks once, and separately simulate 8,192 distinct array layers sharing real
immutable inputs. The latter deliberately exceeds the cache budget and practical
atlas sizes to exercise admission and fallback. Fresh-state samples include
teardown. Source-frame samples report peak retained bytes for one animation;
other samples report the state's retained pixel-buffer capacity.

Every process also compares complete upload streams with the original function
at 517 tick positions, including duplicate ticks, jumps, rewinds and changing mip
limits, using all real animations (including interpolation metadata). This
preserves the existing frame-based path; it does not add or validate GPU blending
of interpolated frames.

## Validation

- `pr_ready.py --base 89b58991bbc02eb79b6965fc4ca56357fe2297d9 --quick`: public tree,
  formatting, workspace clippy, 196 Python tests and 635 client tests passed.
- Native animation parity: 907 fixtures (63 resource animations), including
  258,996 tick states and 203 intentionally rejected inputs, match the pinned client.
- Native mipmap parity: 1,319 cases / 7,914 output lines match exactly.
- Native breaking-model parity: 2,740 cases / 155,928 vertices match within the
  harness tolerance; maximum error `1.40395e-07`.

The full game parity suite was not run. This checkout has no recorded harness
index; `pr_ready --quick` lists that deferred selection, and the relevant native
harnesses above were run explicitly. GPU interpolation pixels and end-to-end FPS
remain outside this evaluation.

## Measured outcome

Kept. Ryzen 5 5600X (12 logical CPUs), rustc 1.99.0
(`b940084d7`, 2026-09-28), Linux; 2,131 prepared block/item textures,
54 animations and 594 distinct source frames. Three paired process runs,
reversing baseline/candidate order in round two, each with one warmup and nine
samples per workload (27 samples per variant). Both binaries used the same
private Cargo dependency resolution and optimized profile. Baseline gfx is
`89b58991bbc02eb79b6965fc4ca56357fe2297d9`.

CPU times below are median [minimum, maximum], in milliseconds. Each repeated
sample averages 1,024 ticks or 3,072 render calls; unique/cold samples include
state setup and teardown.

| Workload | Baseline | Cache | Median change |
| --- | --- | --- | --- |
| Fresh state + first tick + drop | 7.532 [7.193, 7.567] | 7.545 [7.128, 7.791] | +0.18% |
| Repeated tick | 3.627 [3.491, 3.720] | 1.692 [1.646, 1.739] | -53.34% |
| Repeated render, three renders/tick | 1.203 [1.181, 1.244] | 0.562 [0.546, 0.585] | -53.33% |
| Each actual source frame once | 98.168 [96.647, 102.813] | 97.093 [96.674, 104.441] | -1.09% |
| 8,192 distinct layers, first frame | 1123.475 [1088.996, 1156.957] | 1121.300 [1087.745, 1154.184] | -0.19% |

Repeated-tick medians for the three processes were respectively
`3.647 / 3.646 / 3.540` ms baseline and `1.702 / 1.674 / 1.692` ms cached.
The corresponding render-call medians were `1.233 / 1.195 / 1.192` ms and
`0.562 / 0.556 / 0.562` ms. The improvement repeats in every pair; cold and
unique-frame differences are within the observed variation.

Additional per-process samples, baseline → cache:

- Resource decoding: `69.829 / 69.350 / 67.308` → `68.632 / 66.235 / 67.252` ms.
- Process-first setup/upload/drop, including mip table initialization:
  `7.255 / 7.979 / 7.610` → `8.796 / 7.706 / 7.688` ms. The first cached
  process had a cold outlier; no cold-start speedup is claimed.
- Setup plus first 4,096 ticks, including cache fill:
  `14.835 / 15.001 / 14.746` → `7.053 / 6.997 / 6.955` seconds.
- Individual warmed-tick p50: `4.311 / 4.313 / 4.318` →
  `1.639 / 1.543 / 1.550` ms; p95: `6.613 / 6.617 / 6.885` →
  `4.139 / 3.960 / 3.972` ms; maximum: `7.621 / 7.702 / 7.569` →
  `5.285 / 6.448 / 5.213` ms.
- Whole-process peak RSS: `84412 / 83388 / 85416` →
  `84480 / 83820 / 84524` KiB, using a fresh Python parent's
  `resource.getrusage(RUSAGE_CHILDREN).ru_maxrss` per invocation. RSS includes
  resource loading and the synthetic workload, and allocator reuse/other peaks
  hide the retained cache cost; it does **not** establish zero added memory.

At 64px, each generated chain has 32,768 bytes of `Vec` capacity. The cache
retains 256 entries / exactly 8,388,608 bytes, with map capacity 448, and bypasses
the remaining frames. Retaining all 594 frames would require 18.56 MiB of buffer
capacity. The first tick retains 1,769,472 bytes; the one-animation-at-a-time
unique-source workload peaks at 1 MiB. The separate 4,096-entry cap bounds map
metadata for tiny textures. No GPU memory or upload bytes are saved.

All six measured processes passed the 517-position upload-stream comparison.
The 196 Python tests were rerun after adding the build-only driver mode and
passed. The first RSS wrapper attempt failed before collecting samples because
`/usr/bin/time` was unavailable; the complete paired session above uses Python
resource counters. Private raw output remains under `.local/sprite-mips-*`.
