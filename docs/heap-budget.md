# Heap budget experiment

Halving the maximum Java heap from 3 GiB to 1.5 GiB does not halve total process
RAM in this workload. Three repeated core-profile pairs show nearly unchanged
average FPS and variable sampled RSS reductions, with more GC accounting time
in every smaller-heap run. Playable memory recommendations remain unchanged.

## Matched experiment

These runs use the exact published 0.20.0 mod and the same private camera fixture
as the [repeated core comparison](core-performance.md). Both sides use the core
stack, 12 full-detail / 12 simulation chunks, 1280×800, FOV 70, uncapped FPS,
OpenGL, no LOD/shaders, G1 and compact headers. Only maximum heap differs. The
initial heap is 512 MiB in both sides. Every run uses a fresh process and the same
initial world copy, warms for 180 seconds and records for 120 seconds. Each pair
runs 3 GiB first, followed by 1.5 GiB.

The host is an i7-7500U with 2 physical cores / 4 logical processors and about
30.2 GiB total RAM, using Linux llvmpipe software rendering. This is not a test
under low system-memory pressure. Private copies use Peaceful difficulty and
fixed noon/weather advancement rules. The camera is stationary and every
original experimental controller is disabled. Those fixture rules are absent
from playable packs.

## Results

| Pair | 3 GiB FPS | 1.5 GiB FPS | 3 GiB peak RSS | 1.5 GiB peak RSS | 3 GiB p95 frame | 1.5 GiB p95 frame | GC accounting |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 6.4549 | 6.5133 | 2,080.07 MiB | 2,041.48 MiB | 166.26 ms | 173.65 ms | 95 → 162 ms |
| 2 | 6.5528 | 6.5115 | 2,482.75 MiB | 1,997.45 MiB | 166.18 ms | 174.22 ms | 72 → 93 ms |
| 3 | 6.5624 | 6.5556 | 2,242.84 MiB | 1,989.43 MiB | 166.55 ms | 165.82 ms | 71 → 105 ms |

Paired average-FPS changes are +0.90%, −0.63%, −0.10%. Their descriptive
arithmetic mean is +0.06%; this does not establish an FPS improvement. Peak
sampled RSS is 1.86%, 19.55% and 11.30% lower, illustrating substantial process
variation. A smaller configured heap limit is not a total resident-memory cap.
The smaller-heap captures still use roughly 1.94–1.99 GiB of sampled process RSS.

p95 frame intervals are 4.44% and 4.84% worse in the first two pairs and 0.44%
better in the third. Slowest-1%-average FPS changes are −0.06%, −5.08%, +10.03%.
CPU changes are −5.10%, +0.24%, −0.17%, without a consistent absolute reduction.
GC accounting time increases 29–71%; these MXBean totals are not measured
stop-the-world pause durations. Tick execution means change −7.25%, +9.22%,
+1.15%, while observed invocation throughput stays near 20 TPS in every run.

## Validation and limits

All six clients save and exit with code zero. Frame/tick CSV counts and mean
durations match their JSONs, with zero exclusions or dropped samples. Captured
settings, Java/collector/header configuration and loaded mod identities match
apart from the intentional heap maximum. Sampled tick-manager state remains
normal at 20 configured TPS, without freeze, step, sprint or rate changes. Saved
camera, position, clock and weather rules match across runs.

All three actual frame/tick JSON+CSV pairs pass desktop/mobile viewer checks,
including duration traces and the visible intentional heap mismatch. The
[selected observations](../benchmarks/heap-budget-2026-10-04/live-observations.json)
retain all six runs, paired metrics, configuration, software identity and private
capture/fixture hashes. Raw worlds, CSVs and complete logs remain private.

These are sampled peaks during settled recordings, not whole-session startup
peaks or complete GPU/unified-memory accounting. Three fixed-order pairs in one
world cannot establish capacity for exploration, generation, long sessions,
shaders, distant terrain, all hardware or 50 dedicated-server players. JIT, OS
cache, entities and thermal effects can still vary. Do not pool this experiment
with vanilla/core comparisons or infer physical M2 results from it.

The 16 GB Mac starter retains its 6 GiB recommendation for balanced distant
terrain. See the [Mac test procedure](mac-first-test.md) and
[memory measurement limits](process-memory.md) when measuring another budget.
