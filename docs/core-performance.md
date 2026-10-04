# Core performance comparison

`core-performance` adds the pinned core performance mods while retaining the
baseline's 12 full-detail chunks, 12 simulation chunks, suggested heap, custom
graphics preset and OpenGL backend. Neither profile installs distant terrain or
Iris shaders; every original experimental controller is disabled. Use this pair
for the first comparison at the same view distance, then test balanced separately
for its additional 128-chunk distant horizon.

The loaded-mod report also exposes dependencies bundled inside those downloads,
such as Transition and Trender in this observed core run. The comparator treats
these software changes as explicit experiment variables. Matching configuration
is not proof of pixel-identical rendering or exhaustive gameplay equivalence.

## Three repeated pairs on 0.20.0

The later series uses the exact published 0.20.0 mod with fresh JVMs and identical
initial private world copies. Each run warms for 180 seconds in focused gameplay,
then records frames and integrated-server ticks for 120 seconds. A separate test
fixture in both profiles holds the same stationary camera. It is absent from
playable packs. The workload remains 12 full-detail / 12 simulation chunks,
1280×800, FOV 70, no LOD/shaders, G1, compact headers and a 3 GiB maximum heap.
Private copies use Peaceful difficulty and a fixed noon clock/weather; distributed
packs retain normal rules. All original experimental controllers are disabled.

| Pair | Baseline FPS | Core FPS | Baseline peak RSS | Core peak RSS | Baseline CPU | Core CPU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 5.39 | 6.53 | 2,310.68 MiB | 2,043.46 MiB | 2.931 cores | 2.898 cores |
| 2 | 5.55 | 6.32 | 2,572.06 MiB | 2,031.07 MiB | 2.928 cores | 2.981 cores |
| 3 | 5.39 | 6.52 | 2,172.38 MiB | 2,121.73 MiB | 2.942 cores | 2.950 cores |

Paired average-FPS changes are **+21.28%, +14.00%, +20.96%**. Their arithmetic
mean is +18.75%; this is a descriptive average of three changes, not a pooled
frame estimate or a statistical significance claim. The slowest-1%-average rate
is higher in all pairs, while p95 frame intervals are 8.43–21.25% lower. Peak
sampled RSS changes are **−11.56%, −21.03%, −2.33%**. Memory varies substantially
across fresh processes, so the earlier single pair's 22% reduction is not a
stable memory-saving estimate.

Average process CPU changes are −1.11%, +1.81%, +0.30%: there is no consistent
absolute CPU reduction while rendering more frames. Peak sampled JVM heap is
slightly higher in the third core run. GC accounting time is 113→92 ms, 57→154 ms
and 157→96 ms across the three pairs, so it is mixed too. These are collector
MXBean totals, not JFR stop-the-world pause measurements; no JFR profiling runs
alongside these captures.

Every accepted run has nonempty frame/tick CSVs with verified counts and mean
durations, zero excluded frames or dropped samples, matching captured settings
and no visual-setting changes. Sampled tick-manager state remains normal at
20 configured TPS with no observed freeze, step, sprint or rate changes. Observed
invocation throughput ranges from 19.997 to 20.006 TPS; mean tick execution time
is 26.43–32.91% lower with core. This stationary integrated-server workload does
not establish capacity for a populated dedicated server. Saved cameras, positions,
clock/rules and clean process exits are checked for every run. All three real
JSON/CSV pairs pass desktop/mobile comparison and chart checks.

The original third-pair candidate lost focus when the headless input tool's
single long sleep overflowed and its keyboard disappeared. Its 448 eligible
frames, 211 excluded frame starts and 1,391 measured ticks over about 120 seconds
are excluded. The preceding valid baseline is retained separately as unpaired
data. The complete third pair was repeated from fresh copies after replacing
the long sleep with several shorter sleeps in one persistent process and adding
an input-process liveness check. Neither excluded attempt enters the summary.

The [selected repeated observations](../benchmarks/core-client-repeats-2026-10-04/live-observations.json)
include all six accepted runs, the excluded attempts, each pair's comparison,
loaded mod identities and private capture/fixture hashes. This is still one
stationary view on an i7-7500U Linux **llvmpipe software renderer**. Baseline is
always first within each pair. OS cache, JIT, entity activity and thermal effects
remain uncontrolled variables. Three pairs do not prove a universal improvement;
Apple Silicon/native GPU performance, movement and terrain generation remain to
be measured. See the [Mac first-test guide](mac-first-test.md) for a matched-view
comparison on the user's hardware.

## Earlier single pair on 0.19.0

The selected 4 October 2026 live observations use separate private world copies,
a stationary camera, native Linux x64 Java 25, G1, compact headers and the same
3 GiB maximum heap. The test copies use Peaceful difficulty and a noon clock with
weather/time advancement disabled to reduce lighting variation. Distributed packs
do not change world rules. First saved cameras and saved clock/rules are checked.
JFR begins roughly 121 seconds after demo dismissal; a separate 120-second frame
capture begins roughly 307 seconds after dismissal.

| Observation | Vanilla baseline | Core mods |
| --- | ---: | ---: |
| Eligible frames | 651 | 785 |
| Average frame rate | 5.44 FPS | 6.56 FPS |
| 95th-percentile frame interval | 194.89 ms | 163.18 ms |
| Sampled peak process RSS | 2,710.76 MiB | 2,108.96 MiB |
| Average process CPU | 2.81 cores | 2.88 cores |
| Tick execution mean | 8.98 ms | 6.37 ms |
| Recorded tick samples | 2,401 | 2,402 |

In this single fixed-order, software-rendered pair, average FPS is about 20.7%
higher and sampled peak RSS about 22.2% lower with the core stack. Process CPU is
slightly higher while more frames are rendered. Tick execution durations are
lower; their inverse is execution capacity, not observed TPS. Both captures span
about 120 seconds and record approximately 20 tick samples per elapsed second.
These are descriptive observations, not a statistical or universal speedup claim.
They do not isolate which mod caused a difference. Entity activity, completion
of rendering work, cache/JIT and thermal state remain possible variables. RSS
excludes a complete GPU/unified-memory measurement; Apple Silicon performance
remains unmeasured. Hardware graphics measurements remain needed; the separate repeated series above
uses a different warm-up protocol and must not be pooled with this pair.

Preceding JFR counter rates for the server and render thread are lower with the
core stack, but total and longest measured GC pauses are higher in that profile.
Those recordings precede the frame captures, so rates/pauses must not be treated
as if they covered the later frame interval. Selected observations retain both
findings instead of presenting only favorable measurements.

The [selected raw observations](../benchmarks/core-client-2026-10-04/live-observations.json)
retain software identity, protocol and both positive and negative measurements.
Both clients save and exit with code zero; the core world reloads and saves again.
Actual JSON/CSV viewer and mobile checks pass. Windows, macOS ARM and Linux builds
pass, with byte-identical local and independent Linux production JARs. These are
platform build checks, not game graphics tests on each platform.
