# First test on the M2 Pro Mac

For the 16 GB M2 Pro, start with the published 0.20.0 packs. Core and balanced
pass title/demo-world rendering and save/exit probes on the hosted macOS ARM64
software renderer. Gameplay and FPS on this physical Mac remain unmeasured.
The Linux software-rendering observations do not predict an M2 speedup.

## Import after installing Prism

1. Download [horizon-mac-starter-0.20.0.zip](https://github.com/ludovic111/minecraft-optimization-lab/releases/download/v0.20.0/horizon-mac-starter-0.20.0.zip)
   and extract it in Finder.
2. In Prism, choose **Add Instance > Import**, then select
   `horizon-m2-balanced-0.20.0.mrpack` from the extracted folder. The individual
   `.mrpack` is the import file; the starter ZIP contains several different packs.
   [Prism's import guide](https://prismlauncher.org/wiki/help-pages/zip-import/)
   describes the supported archive formats.
3. Sign into your Microsoft account inside Prism, using your own Java Edition
   license. This project needs no account credentials.
4. Open Prism's global **Settings > Java** and enable **Autodetect Java version**
   and **Auto-download Mojang Java**. This pack requires Java 25. On Apple Silicon,
   use ARM64/aarch64 Java. If you select Java manually, use the instance's
   **Edit > Settings > Java > Java installation > Auto-Detect** and select native
   Java 25. [Prism's Java guide](https://prismlauncher.org/wiki/getting-started/installing-java/)
   explains automatic downloads and manual selection.
5. In the instance's **Edit > Settings > Java**, enable the Memory override and
   set maximum memory to **6144 MiB**, minimum to **512 MiB**. This is a starting
   heap budget, not a measurement of total process or unified GPU memory.
6. Launch and create a new test world, or use a copy of a Minecraft 26.3 world.
   Save, quit, reopen and check normal movement, breaking/placing blocks and
   inventory interaction before benchmarking. Start at a modest window size,
   such as 1280×800; the recorder reports the actual framebuffer dimensions,
   which may differ from the logical window size on a Retina display.

Balanced uses 12 full-detail chunks and a 128-chunk distant terrain radius.
Allow the distant cache to settle. Distant terrain uses simplified geometry and
does not extend normal simulation. The cinematic and Vulkan packs are separate
experiments to try after this first launch succeeds.

## Measure performance at the same view distance

Import `horizon-baseline-0.20.0.mrpack` and
`horizon-core-performance-0.20.0.mrpack` from the same starter folder. Both use
12 full-detail chunks, 12 simulation chunks and no distant LOD or shaders. Core
adds the pinned performance mods; all original experimental controllers are off.
Comparing these two isolates the core stack more closely than comparing baseline
with balanced's additional distant terrain.

Use identical separate world copies, the same Java/heap/JVM flags, actual
framebuffer size, FOV, graphics quality, VSync, FPS limit and camera route. Close
other game instances, plug in the Mac and keep its power mode consistent. Use
uncapped FPS with VSync off for both measurements if you want to measure available
throughput; return to a display-appropriate cap or VSync for everyday play.

Warm each run for at least three minutes after entering the world and dismissing
menus. Longer generation workloads may need more time. Press **F8** (possibly
**Fn+F8**) in focused gameplay for a 120-second recording. Keep the view or route
consistent and avoid menus during recording. After it finishes, save and quit.
Repeat three pairs in this order:

| Pair | First run | Second run |
| --- | --- | --- |
| 1 | baseline | core-performance |
| 2 | baseline | core-performance |
| 3 | baseline | core-performance |

This alternates profiles but always puts baseline first within a pair. It reduces
dependence on one run; it does not remove order, cache or thermal effects. Keep
fresh terrain exploration separate from already-generated terrain. Also test
normal gameplay after the stationary view; matching settings is not exhaustive
gameplay or visual equivalence.

## Read the results

Open each instance's Minecraft folder in Prism and find `horizon-benchmarks/`.
Keep each run's frame JSON/CSV and tick JSON/CSV together. Open
`Compare-Benchmarks.html` from the starter folder and select one baseline frame
JSON plus its CSV and the paired core frame JSON plus its CSV. Then compare the
paired tick files separately. The viewer works locally without uploading files.

Check settings mismatches, exclusions, dropped samples and tick-state warnings
before comparing FPS, p95/p99 frame intervals, peak sampled RSS and average process
CPU. Compare all three pairs rather than selecting the best result. Normal tick
invocation throughput should remain near 20 TPS; inverse tick execution duration
is not TPS. RSS is sampled once per second and does not cover all unified GPU
memory. See [tick-state limits](tick-state.md) and [memory limits](process-memory.md).

Finally test balanced for the longer horizon using the same route and resolution,
with a settled distant cache. Treat its extra view distance as a changed visual
workload, then decide whether the appearance and performance meet your goal.
