# Matched graphics API comparison

Three core-profile pairs compare OpenGL with Vulkan at the same stationary view.
On this Linux software renderer, Vulkan produces only 1.22–1.74% more average FPS
while using 11.23–12.95% more process CPU. Frame-time tails and sampled RSS are
mixed, and integrated-server tick execution is slower. Playable backend defaults
remain unchanged. These results do not predict physical M2 GPU performance.

## Protocol

Each fresh process uses the exact published 0.20.0 mod, identical core mods,
private camera fixture, initial world copy and 3 GiB maximum / 512 MiB initial
heap. Both APIs use 12 full-detail / 12 simulation chunks, 1280×800, FOV 70,
uncapped FPS, no LOD/shaders, G1 and compact headers. Every original experimental
controller is disabled. The private camera is fixed and world copies use Peaceful
difficulty with noon/weather advancement rules fixed. Playable packs retain
normal rules.

Runs warm for 180 seconds, then record frames and ticks for 120 seconds. Each
pair runs OpenGL first and Vulkan second. Actual devices report llvmpipe LLVM
21.1.8, with Mesa 26.0.8 OpenGL 4.5 or Vulkan 1.4.335. The Vulkan preference is
verified against the observed backend; a fallback would invalidate this series.
The host is an i7-7500U with 2 physical cores / 4 logical processors and about
30.2 GiB system RAM. Native GPU access is unavailable to this user.

## Results

| Pair | OpenGL FPS | Vulkan FPS | OpenGL CPU | Vulkan CPU | OpenGL peak RSS | Vulkan peak RSS | Tick mean, OpenGL → Vulkan |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 6.5786 | 6.6765 | 2.895 cores | 3.248 cores | 2,362.36 MiB | 2,337.00 MiB | 6.455 → 7.460 ms |
| 2 | 6.5869 | 6.6671 | 2.918 cores | 3.246 cores | 2,471.90 MiB | 2,526.52 MiB | 6.845 → 7.602 ms |
| 3 | 6.5632 | 6.6775 | 2.879 cores | 3.252 cores | 2,424.70 MiB | 2,459.63 MiB | 6.499 → 7.593 ms |

Paired FPS changes are +1.49%, +1.22%, +1.74%, with a descriptive arithmetic
mean of +1.48%. CPU changes are +12.18%, +11.23%, +12.95% (mean +12.12%).
These three fixed-order pairs do not establish statistical significance.

p95 frame intervals change −2.08%, +0.94%, −1.43%; slowest-1%-average FPS
changes −2.19%, −7.63%, +5.49%. Peak sampled RSS changes −1.07%, +2.21%,
+1.44%. GC accounting is mixed and is not a measured stop-the-world pause total.
Tick execution means increase 11.06–16.84%, while observed invocation throughput
remains near 20 TPS with normal sampled tick-manager state in all runs.

Process CPU includes native rendering and the integrated server. Higher tick
durations in a shared rendering process do not isolate a server algorithm change
or establish dedicated-server capacity. Tick reports do not capture the client
graphics API; their captured-settings match must be interpreted alongside each
paired frame report's explicit backend/driver differences.

## Validation and limits

All six clients save and exit cleanly. Raw frame/tick CSV counts and mean durations
match the JSONs, with zero exclusions/drops. Runtime and fixture hashes, mod
identities, other captured settings, saved camera/position and clock/weather rules
match. Frame comparisons retain only the deliberate backend/driver mismatches.
Both first-pair warmed views were visually inspected before recording and showed
the same forest/terrain. This is not an exhaustive pixel-equivalence test.

All three actual frame/tick JSON+CSV pairs pass desktop/mobile viewer and chart
checks; representative screenshots were inspected. The
[selected observations](../benchmarks/graphics-api-2026-10-04/live-observations.json)
retain all six runs, every paired metric, configuration, device identities and
private capture/fixture hashes. Raw logs, CSVs, images and worlds remain private.

One stationary view, a software renderer, fixed order and three pairs cannot
establish movement/generation behavior, native GPU or Mac speedups, full visual
equivalence or populated-server capacity. OS cache, JIT, entities and thermal
effects can still vary. Sampled RSS covers the settled recording, can miss spikes
and is not complete GPU/unified-memory accounting. Do not pool these API pairs
with heap or vanilla/core experiments. See the [Mac procedure](mac-first-test.md)
for physical-device comparisons and the [graphics probes](client-graphics-smoke.md)
for separate hosted-Mac startup results.
