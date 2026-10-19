# Java performance packs (no longer developed)

Before the Rust rewrite, Horizon was a lab for Minecraft Java client performance: pinned Fabric performance packs, distant terrain, an original frame and tick recorder, and client and server starter downloads (the server starter is gone: [horizon-server](hosting.md) replaces it). This track is **no longer developed**. No release downloads are published; build the packs and starters locally with `python3 tools/build_release.py` (see [Build and run](#build-and-run)). This page keeps its documentation.

## Java packs: Mac starter (out of scope)

Build **horizon-mac-starter-0.21.0.zip** with `python3 tools/build_release.py` (it lands in `dist/`), extract it and open `Mac-Start.command`. It verifies and installs official universal Prism Launcher in `~/Applications` if needed, then opens the balanced pack import. Sign into Microsoft **inside Prism yourself**. This project needs no credentials.

Use native **Java 25 ARM64**, maximum heap **6144 MiB**, minimum **512 MiB** on a 16 GB M2 Pro Mac. Prism can download Java automatically. Start at 1280×800 or 1920×1200; full Retina resolution is a larger GPU workload. [START-HERE.txt](../packs/START-HERE.txt) has details. The same `.mrpack` files import into Prism on Windows and Linux.

Core and balanced passed macOS title and demo-world rendering probes on the hosted
ARM64 software renderer, including readback, saves and clean exits. Physical M2
GPU performance and macOS 27 were never tested.

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

The profiles marked *experiment* each isolate one change to Distant Horizons,
EntityCulling or the JVM (adaptive terrain throttling, precise worker pacing,
debounce-queue waits, rejected-dispatch retries, empty culling whitelists,
culling ray scratch vectors, terrain quad reuse, terrain position arithmetic,
tick membership storage, a smaller heap, compact object headers). Each kept the
upstream behaviour it replaced and was measured only on Linux software rendering,
where none showed a gain large enough to enable by default; balanced keeps them
all off. Their write-ups were removed once the Rust rewrite replaced this track;
they remain in Git history. Matched software-rendered pairs showed `core-performance`
14–21% faster than `baseline`, and Vulkan about 1.5% more FPS for about 12% more
CPU; neither is evidence for a Mac GPU.

To try the game's own Vulkan backend on a Mac, see [Vulkan on your Mac](vulkan.md);
to compare garbage collectors, see [collector comparisons](collectors.md).

## Measure on your hardware

Press **F8** during gameplay to start a 120-second recording, or again to stop early. Rebind in Controls if needed. This also works with restricted chat accounts. Alternatively: `/horizon bench <label> 120`, `/horizon stop`, `/horizon status`.

Use separate identical world copies and the same camera route, resolution, render/simulation distance, heap, VSync and FPS limit. Warm up, then alternate at least three baseline/optimized runs. Test fresh chunk exploration separately. For everyday play cap FPS or enable VSync to reduce power usage.

The instance's `horizon-benchmarks/` folder receives JSON/CSV frame and integrated-server tick timings: average FPS, slowest-1%-average FPS, p95/p99 durations, stalls, JVM heap/GC, sampled process RSS when available, GPU/backend/driver and render settings. Frames include presentation/limiting waits and exclude menus/pauses/unfocused time. Tick `meanRate` is execution capacity, **not TPS**; observed tick counts are separate. Version 0.20 [captures sampled tick-manager state](tick-state.md), including restored freeze/step/sprint/rate changes; this does not measure completed world work. Resident process memory is sampled on a shared background thread only during recordings; it includes native resident pages, while heap measures JVM objects. Neither is a complete measurement of unified GPU memory. [Memory measurement details](process-memory.md).

```sh
python3 tools/compare_benchmarks.py baseline-frames.json optimized-frames.json --strict
```

Open `Compare-Benchmarks.html` from the starter ZIP or [the source file](../tools/Compare-Benchmarks.html) in your browser to compare without Python. Select each JSON and optionally its matching CSV. It exports summary JSON, flags settings and duration differences, and charts frames by recorded active time or ticks by sample number. It is self-contained and makes no network requests.



The comparison flags captured setting differences, but cannot verify route, world, shader state or cache warm-up. Software-renderer test FPS is not evidence of a Mac/GPU speedup.

For CPU/allocation investigation, [capture and stream a private JFR summary](profiling.md).
It distinguishes directly observed allocation counters from sampled hot methods;
profiling runs remain separate from normal timing comparisons.

## Build and run

Requires Python 3.10+ and JDK 25. No Gradle, Rust or native compilation is required for playable packs.

```sh
python3 tools/build_mod.py
python3 tools/build_release.py
```

Gitignored `dist/` contains twenty packs, our original instrumentation JAR, the client starter ZIP and SHA256SUMS. [mods.lock.json](../packs/mods.lock.json) pins mod versions, exact dependency closure, SHA-1/SHA-512 and source/license; [fabric.lock.json](../packs/fabric.lock.json) pins loader/library SHA-256. Packs use the [Modrinth format](https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack). They embed only original project code; third-party mods, game and assets download from their publishers.

The Fabric server starter (Lithium/FerriteCore, `tools/server.py`) was retired on 2026-10-19. To host a server, use [horizon-server](hosting.md) ([playhorizon.dev/server](https://playhorizon.dev/server)); its benchmarks are in [server.md](server.md), section 9.

Local smoke testing uses the game's built-in limited demo; normal play uses Prism's licensed account authentication:

```sh
python3 tools/build_pack.py --profile m2-balanced --instance .local/runtime/m2-balanced
vscreen run python3 tools/runtime.py demo --instance .local/runtime/m2-balanced
```

The standalone demo launcher applies the pinned official manifest's conditional
JVM flags, including macOS `-XstartOnFirstThread` and the Windows heap-dump-path
option. Prism applies its own official manifest. Server JVM policy is separate;
this launcher correction is not a measured graphics performance improvement.

