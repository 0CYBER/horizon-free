# Java performance packs (no longer developed)

Before the Rust rewrite, Horizon was a lab for Minecraft Java client performance: pinned Fabric performance packs, distant terrain, an original frame and tick recorder, and client and server starter downloads. This track is **no longer developed**. No release downloads are published; build the packs and starters locally with `python3 tools/build_release.py` (see [Build and run](#build-and-run)). This page keeps its documentation.

## Java packs: Mac starter (out of scope)

Build **horizon-mac-starter-0.21.0.zip** with `python3 tools/build_release.py` (it lands in `dist/`), extract it and open `Mac-Start.command`. It verifies and installs official universal Prism Launcher in `~/Applications` if needed, then opens the balanced pack import. Sign into Microsoft **inside Prism yourself**. This project needs no credentials.

Use native **Java 25 ARM64**, maximum heap **6144 MiB**, minimum **512 MiB** on a 16 GB M2 Pro Mac. Prism can download Java automatically. Start at 1280×800 or 1920×1200; full Retina resolution is a larger GPU workload. [START-HERE.txt](../packs/START-HERE.txt) has details. The same `.mrpack` files import into Prism on Windows and Linux.

If Prism is already installed, follow the [Mac import and first comparison guide](mac-first-test.md).

Core and balanced also pass [macOS title/demo-world rendering probes](client-graphics-smoke.md)
on the hosted ARM64 software renderer, including readback, saves and clean exits.
Physical M2 GPU performance and macOS 27 remain to be tested.

| Profile | Full-detail chunks | Distant LOD radius | Suggested maximum heap |
| --- | ---: | ---: | ---: |
| baseline | 12 | none | 6144 MiB |
| core-performance | 12 | none | 6144 MiB |
| m2-balanced | 12 | 128 | 6144 MiB |
| adaptive (experiment) | 12 | 128 | 6144 MiB |
| worker-pacing (experiment) | 12 | 128 | 6144 MiB |
| adaptive-pacing (experiment) | 12 | 128 | 6144 MiB |
| queue-wait (experimental) | 12 | 128 | 6144 MiB |
| adaptive-queue (experimental) | 12 | 128 | 6144 MiB |
| dispatch-retry (experimental) | 12 | 128 | 6144 MiB |
| tick-dedup (experimental) | 12 | 128 | 6144 MiB |
| culling-empty (experimental) | 12 | 128 | 6144 MiB |
| quad-reuse (experimental) | 12 | 128 | 6144 MiB |
| terrain-position (experimental) | 12 | 128 | 6144 MiB |
| culling-rays (experimental) | 12 | 128 | 6144 MiB |
| foreground-first (combined experiment) | 12 | 128 | 6144 MiB |
| foreground-vulkan (combined experiment) | 12 | 128 | 6144 MiB |
| low-memory | 8 | none | 3072 MiB |
| far-distance | 12 | 256 | 8192 MiB, with sufficient physical RAM |
| cinematic | 12 | 128 | 6144 MiB |
| vulkan | 12 | 128 | 6144 MiB |

Balanced combines [Sodium](https://modrinth.com/mod/sodium), [Lithium](https://modrinth.com/mod/lithium), [FerriteCore](https://modrinth.com/mod/ferrite-core), [ImmediatelyFast](https://modrinth.com/mod/immediatelyfast), [EntityCulling](https://modrinth.com/mod/entityculling), and [Distant Horizons](https://modrinth.com/mod/distanthorizons). Cinematic adds [Iris](https://modrinth.com/mod/iris) and [Complementary Reimagined](https://modrinth.com/shader/complementary-reimagined), initially disabled. Vulkan independently experiments with the game's own backend and fallback, without Iris.

Distant LOD is simplified terrain. Nearby blocks retain full detail. Initial generation costs CPU/disk/memory and needs warm-up. DH adjusts clouds, chunk fade and transparency to blend its renderer; assess appearance as well as speed. Shaders cost GPU time and have [Mac-specific limits](https://shaders.properties/current/reference/miscellaneous/macos/). Universal CPU/GPU support and exact vanilla equivalence remain validation goals. Simulation distance stays 12 in every profile; normal 20 TPS rules are retained.

For the first comparison at the same view distance, use `baseline` versus
`core-performance`: both select 12 full-detail chunks and no distant LOD or shaders.
The latter adds only the pinned core performance mods and leaves every original
controller disabled. Then test balanced separately for its additional distant
terrain. Matching captured settings does not prove pixel-identical rendering.
[Core comparison protocol and selected observations](core-performance.md)
include three repeated matched-view software-rendered pairs with 14–21% higher
FPS in this workload; M2 performance remains unmeasured.

[Heap budget experiment](heap-budget.md): three pairs with 3 GiB versus
1.5 GiB maximum heap show variable RAM savings and GC/frame-time tradeoffs;
playable defaults remain unchanged.

[Matched graphics API comparison](graphics-api-comparison.md): Vulkan adds
about 1.5% average FPS but about 12% CPU in three Linux software-rendered pairs;
backend defaults remain unchanged and physical Mac performance is unmeasured.

The optional `queue-wait` pack yields during pending distant-terrain debounce retries; `adaptive-queue` combines it with the existing controllers. [Queue experiment and measurement limits](debounce-queue.md).

Version 0.16 adds a separate [terrain position arithmetic experiment](terrain-position.md)
for the sampled full-data lookup. It retains the pinned library’s unusual
negative-coordinate rounding and invalid-level errors. Balanced keeps it off;
whole-game and Mac performance still require measurement.

Version 0.15 adds an isolated [culling ray scratch experiment](culling-rays.md)
that reuses two numeric vectors within each culling instance while retaining
upstream arithmetic and decisions. Balanced keeps it off; hardware gains remain
unmeasured.

Version 0.14 adds an isolated [empty culling whitelist experiment](culling-empty.md)
that avoids allocating iterators while there are no dynamic handlers. Live
registration and nonempty callbacks retain upstream behavior. Balanced keeps it
off; whole-game gains still need hardware comparisons. DH profiles also request
a scoped [interruptible blending-read wait](terrain-read-wait.md) to let
background workers leave a pending metadata read during shutdown.

Version 0.13 adds an isolated [tick membership experiment](tick-membership.md)
that reuses compact storage for DH's private per-tick membership checks. The
combined foreground profiles include it too; balanced keeps it off. The applied
holder test verifies action semantics and removes repeated entry allocations;
whole-game FPS and RAM gains still need hardware measurements.

Version 0.12 captures effective distant-terrain quality, radius, worker budgets,
Iris state and field of view. Start/end snapshots flag changed visual settings;
[measurement details](terrain-measurements.md) explain their limits.

Version 0.11 adds combined foreground-first OpenGL/Vulkan profiles with a 120 FPS
adaptive goal, plus explicit collector experiments and GC measurement guidance.
See [combined controls](foreground-first.md), [Vulkan on your Mac](vulkan.md)
and [collector comparisons](collectors.md). The frame goal is not a rendering cap.

Version 0.10 adds an isolated [rejected-dispatch retry experiment](dispatch-retry.md).
It bounds retry passes when a DH worker rejects queued work, preserving original
requeue handling and allowing other executors to progress. Balanced keeps it off.

Version 0.9 also makes the queue's idle wait wakeable for incoming work, preserving
its debounce rules and bounded cache-maintenance cadence. It remains part of the
optional queue profiles, with separate wake-latency measurements.

Version 0.8 fixes a fresh-instance graphics preset that could override the selected
render distance. It also records actual GC/header settings and supports an optional
[compact object headers experiment](compact-headers.md) for Java 25; packs
do not enable that JVM flag automatically.

## Measure on your hardware

The optional [adaptive terrain experiment](adaptive-terrain.md) throttles distant
terrain background work when frames slow down or heap pressure rises, keeping the
balanced profile's view distances and simulation settings. It remains opt-in until
real hardware demonstrates a benefit.

The additional [precise worker pacing experiment](worker-pacing.md) makes short
terrain tasks respect their duty-cycle budget and avoids boxed worker timestamps.
It changes generation speed and CPU use, so it stays separate from balanced.

Press **F8** during gameplay to start a 120-second recording, or again to stop early. Rebind in Controls if needed. This also works with restricted chat accounts. Alternatively: `/horizon bench <label> 120`, `/horizon stop`, `/horizon status`.

Use separate identical world copies and the same camera route, resolution, render/simulation distance, heap, VSync and FPS limit. Warm up, then alternate at least three baseline/optimized runs. Test fresh chunk exploration separately. For everyday play cap FPS or enable VSync to reduce power usage.

The instance's `horizon-benchmarks/` folder receives JSON/CSV frame and integrated-server tick timings: average FPS, slowest-1%-average FPS, p95/p99 durations, stalls, JVM heap/GC, sampled process RSS when available, GPU/backend/driver and render settings. Frames include presentation/limiting waits and exclude menus/pauses/unfocused time. Tick `meanRate` is execution capacity, **not TPS**; observed tick counts are separate. Version 0.20 [captures sampled tick-manager state](tick-state.md), including restored freeze/step/sprint/rate changes; this does not measure completed world work. Resident process memory is sampled on a shared background thread only during recordings; it includes native resident pages, while heap measures JVM objects. Neither is a complete measurement of unified GPU memory. [Memory measurement details](process-memory.md).

```sh
python3 tools/compare_benchmarks.py baseline-frames.json optimized-frames.json --strict
```

Open `Compare-Benchmarks.html` from the starter ZIP or [the source file](../tools/Compare-Benchmarks.html) in your browser to compare without Python. Select each JSON and optionally its matching CSV. It exports summary JSON, flags settings and duration differences, and charts frames by recorded active time or ticks by sample number. It is self-contained and makes no network requests.

Version 0.18 adds an isolated [terrain quad reuse experiment](quad-reuse.md).
It reuses a temporary quad discarded by successful premerge when the existing
pool would otherwise allocate. Generated geometry and thread ownership checks
pass; allocation results are not whole-game RAM or FPS evidence. It remains off
in combined profiles.

Version 0.17 also records loaded mod versions and the Java vendor, VM and runtime
build in client and server reports. Both comparison tools flag software changes
as explicit experiment variables; older reports remain usable with an unknown
software warning. The viewer can expand the complete loaded mod list.
[Live software identity check](../benchmarks/software-identity-2026-10-04/live-observations.json)
records the metadata and file-count validation; its short capture is not a
performance comparison.


The comparison flags captured setting differences, but cannot verify route, world, shader state or cache warm-up. See [validation evidence](playable-validation.md). Software-renderer test FPS is not evidence of a Mac/GPU speedup.

For CPU/allocation investigation, [capture and stream a private JFR summary](profiling.md).
It distinguishes directly observed allocation counters from sampled hot methods;
profiling runs remain separate from normal timing comparisons.

## Build and run

Requires Python 3.10+ and JDK 25. No Gradle, Rust or native compilation is required for playable packs.

```sh
python3 tools/build_mod.py
python3 tools/build_release.py
```

Gitignored `dist/` contains twenty packs, our original instrumentation JAR, client/server starter ZIPs and SHA256SUMS. [mods.lock.json](../packs/mods.lock.json) pins mod versions, exact dependency closure, SHA-1/SHA-512 and source/license; [fabric.lock.json](../packs/fabric.lock.json) pins loader/library SHA-256. Packs use the [Modrinth format](https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack). They embed only original project code; third-party mods, game and assets download from their publishers.

The server starter uses Lithium/FerriteCore and `/horizonserver bench <label> <seconds>`. [Server instructions](server-quickstart.txt) cover setup and a baseline. Files are prepared before you accept Minecraft's EULA locally. A 50-active-player workload remains to be measured.

Local smoke testing uses the game's built-in limited demo; normal play uses Prism's licensed account authentication:

```sh
python3 tools/build_pack.py --profile m2-balanced --instance .local/runtime/m2-balanced
vscreen run python3 tools/runtime.py demo --instance .local/runtime/m2-balanced
```

The standalone demo launcher applies the pinned official manifest's conditional
JVM flags, including macOS `-XstartOnFirstThread` and the Windows heap-dump-path
option. Prism applies its own official manifest. Server JVM policy is separate;
this launcher correction is not a measured graphics performance improvement.

