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
remains unmeasured. Hardware graphics and repeated alternating trials are needed.

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
