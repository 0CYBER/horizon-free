# Minecraft Optimization Lab / Horizon Lab

Client-first performance research for Minecraft Java **26.3**, preserving vanilla gameplay rules. Any language or renderer may be used when measured gains justify it. Not affiliated with Mojang or Microsoft.

**First playable release:** pinned Java performance packs, distant terrain, an original frame/tick recorder, and Mac/client/server starter downloads. The M2 Pro/16 GB Mac is the first testing target. A Rust rewrite or custom renderer has not been completed; no M2 FPS improvement has been measured yet.

## Play on your Mac

Download **horizon-mac-starter-0.16.0.zip** from [Releases](https://github.com/ludovic111/minecraft-optimization-lab/releases), extract it and open `Mac-Start.command`. It verifies and installs official universal Prism Launcher in `~/Applications` if needed, then opens the balanced pack import. Sign into Microsoft **inside Prism yourself**. This project needs no credentials.

Use native **Java 25 ARM64**, maximum heap **6144 MiB**, minimum **512 MiB** on your 16 GB M2 Pro. Prism can download Java automatically. Start at 1280×800 or 1920×1200; full Retina resolution is a larger GPU workload. [START-HERE.txt](packs/START-HERE.txt) has details. The same `.mrpack` files import into Prism on Windows and Linux.

| Profile | Full-detail chunks | Distant LOD radius | Suggested maximum heap |
| --- | ---: | ---: | ---: |
| baseline | 12 | none | 6144 MiB |
| m2-balanced | 12 | 128 | 6144 MiB |
| adaptive (experiment) | 12 | 128 | 6144 MiB |
| worker-pacing (experiment) | 12 | 128 | 6144 MiB |
| adaptive-pacing (experiment) | 12 | 128 | 6144 MiB |
| queue-wait (experimental) | 12 | 128 | 6144 MiB |
| adaptive-queue (experimental) | 12 | 128 | 6144 MiB |
| dispatch-retry (experimental) | 12 | 128 | 6144 MiB |
| tick-dedup (experimental) | 12 | 128 | 6144 MiB |
| culling-empty (experimental) | 12 | 128 | 6144 MiB |
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

The optional `queue-wait` pack yields during pending distant-terrain debounce retries; `adaptive-queue` combines it with the existing controllers. [Queue experiment and measurement limits](docs/debounce-queue.md).

Version 0.16 adds a separate [terrain position arithmetic experiment](docs/terrain-position.md)
for the sampled full-data lookup. It retains the pinned library’s unusual
negative-coordinate rounding and invalid-level errors. Balanced keeps it off;
whole-game and Mac performance still require measurement.

Version 0.15 adds an isolated [culling ray scratch experiment](docs/culling-rays.md)
that reuses two numeric vectors within each culling instance while retaining
upstream arithmetic and decisions. Balanced keeps it off; hardware gains remain
unmeasured.

Version 0.14 adds an isolated [empty culling whitelist experiment](docs/culling-empty.md)
that avoids allocating iterators while there are no dynamic handlers. Live
registration and nonempty callbacks retain upstream behavior. Balanced keeps it
off; whole-game gains still need hardware comparisons. DH profiles also request
a scoped [interruptible blending-read wait](docs/terrain-read-wait.md) to let
background workers leave a pending metadata read during shutdown.

Version 0.13 adds an isolated [tick membership experiment](docs/tick-membership.md)
that reuses compact storage for DH's private per-tick membership checks. The
combined foreground profiles include it too; balanced keeps it off. The applied
holder test verifies action semantics and removes repeated entry allocations;
whole-game FPS and RAM gains still need hardware measurements.

Version 0.12 captures effective distant-terrain quality, radius, worker budgets,
Iris state and field of view. Start/end snapshots flag changed visual settings;
[measurement details](docs/terrain-measurements.md) explain their limits.

Version 0.11 adds combined foreground-first OpenGL/Vulkan profiles with a 120 FPS
adaptive goal, plus explicit collector experiments and GC measurement guidance.
See [combined controls](docs/foreground-first.md), [Vulkan on your Mac](docs/vulkan.md)
and [collector comparisons](docs/collectors.md). The frame goal is not a rendering cap.

Version 0.10 adds an isolated [rejected-dispatch retry experiment](docs/dispatch-retry.md).
It bounds retry passes when a DH worker rejects queued work, preserving original
requeue handling and allowing other executors to progress. Balanced keeps it off.

Version 0.9 also makes the queue's idle wait wakeable for incoming work, preserving
its debounce rules and bounded cache-maintenance cadence. It remains part of the
optional queue profiles, with separate wake-latency measurements.

Version 0.8 fixes a fresh-instance graphics preset that could override the selected
render distance. It also records actual GC/header settings and supports an optional
[compact object headers experiment](docs/compact-headers.md) for Java 25; packs
do not enable that JVM flag automatically.

## Measure on your hardware

The optional [adaptive terrain experiment](docs/adaptive-terrain.md) throttles distant
terrain background work when frames slow down or heap pressure rises, keeping the
balanced profile's view distances and simulation settings. It remains opt-in until
real hardware demonstrates a benefit.

The additional [precise worker pacing experiment](docs/worker-pacing.md) makes short
terrain tasks respect their duty-cycle budget and avoids boxed worker timestamps.
It changes generation speed and CPU use, so it stays separate from balanced.

Press **F8** during gameplay to start a 120-second recording, or again to stop early. Rebind in Controls if needed. This also works with restricted chat accounts. Alternatively: `/horizon bench <label> 120`, `/horizon stop`, `/horizon status`.

Use separate identical world copies and the same camera route, resolution, render/simulation distance, heap, VSync and FPS limit. Warm up, then alternate at least three baseline/optimized runs. Test fresh chunk exploration separately. For everyday play cap FPS or enable VSync to reduce power usage.

The instance's `horizon-benchmarks/` folder receives JSON/CSV frame and integrated-server tick timings: average FPS, slowest-1%-average FPS, p95/p99 durations, stalls, JVM heap/GC, sampled process RSS when available, GPU/backend/driver and render settings. Frames include presentation/limiting waits and exclude menus/pauses/unfocused time. Tick `meanRate` is execution capacity, **not TPS**; observed tick counts are separate. Resident process memory is sampled on a shared background thread only during recordings; it includes native resident pages, while heap measures JVM objects. Neither is a complete measurement of unified GPU memory. [Memory measurement details](docs/process-memory.md).

```sh
python3 tools/compare_benchmarks.py baseline-frames.json optimized-frames.json --strict
```

Open `Compare-Benchmarks.html` from the starter ZIP or [the source file](tools/Compare-Benchmarks.html) in your browser to compare without Python. Select each JSON and optionally its matching CSV. It exports summary JSON, flags settings and duration differences, and charts frames by recorded active time or ticks by sample number. It is self-contained and makes no network requests.

The comparison flags captured setting differences, but cannot verify route, world, shader state or cache warm-up. See [validation evidence](docs/playable-validation.md). Software-renderer test FPS is not evidence of a Mac/GPU speedup.

For CPU/allocation investigation, [capture and stream a private JFR summary](docs/profiling.md).
It distinguishes directly observed allocation counters from sampled hot methods;
profiling runs remain separate from normal timing comparisons.

## Build and run

Requires Python 3.10+ and JDK 25. No Gradle, Rust or native compilation is required for playable packs.

```sh
python3 tools/build_mod.py
python3 tools/build_release.py
```

Gitignored `dist/` contains fourteen packs, our original instrumentation JAR, client/server starter ZIPs and SHA256SUMS. [mods.lock.json](packs/mods.lock.json) pins mod versions, exact dependency closure, SHA-1/SHA-512 and source/license; [fabric.lock.json](packs/fabric.lock.json) pins loader/library SHA-256. Packs use the [Modrinth format](https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack). They embed only original project code; third-party mods, game and assets download from their publishers.

The server starter uses Lithium/FerriteCore and `/horizonserver bench <label> <seconds>`. [Server instructions](docs/server-quickstart.txt) cover setup and a baseline. Files are prepared before you accept Minecraft's EULA locally. A 50-active-player workload remains to be measured.

Local smoke testing uses the game's built-in limited demo; normal play uses Prism's licensed account authentication:

```sh
python3 tools/build_pack.py --profile m2-balanced --instance .local/runtime/m2-balanced
vscreen run python3 tools/runtime.py demo --instance .local/runtime/m2-balanced
```

## Local game inspection

Game JARs, decompiled Java, libraries, inventories, worlds and logs live **inside this checkout** under gitignored **`.local/`**. No game code is in Git history or release downloads. Fresh clones regenerate the inspection workspace from official downloads; Microsoft credentials are not needed for this step.

```sh
git clone https://github.com/ludovic111/minecraft-optimization-lab.git
cd minecraft-optimization-lab
git config core.hooksPath .githooks
python3 tools/lab.py prepare
python3 tools/lab.py decompile
python3 tools/lab.py status
```

`research-lock.json` pins the official manifest and Vineflower. The inner server JAR is extracted and checked; client/server libraries provide decompiler context. Output: `.local/minecraft/26.3/sources/{client,server}/` and coverage/inventory reports. Current inventories cover 11,383 client classes and 7,762 server classes, producing 7,301/5,037 Java files. See [decompilation status](docs/decompilation-status.md).

Class-file SourceFile metadata covers inner classes, which often share a source file. Coverage does not prove semantic equivalence or recompilability; original bytecode remains the reference. Completed source directories are not overwritten automatically. Older obfuscated releases require a separate remapping stage.

## Original Rust research

```sh
cargo test --workspace
cargo run --release --bin voxel-bench -- 100 > .local/voxel-benchmark.csv
```

The dependency-free occupancy experiment compares scalar and bitset exposed-face counting for synthetic 16³ sections. It has no real mesh, material/AO/light work, neighboring-section context, JNI bridge or game integration. Speedups are kernel-only, not Minecraft FPS. [Research plan](docs/optimization-plan.md) describes profiling, improved Java baselines, native integration and behavior checks before expanding scope.

## Validation and ownership

```sh
python3 -m unittest discover -s tests -v
python3 tools/build_mod.py --test-only
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 tools/check_public_tree.py
python3 tools/check_public_tree.py --history
```

CI checks offline tools/Rust and compiles the original mod/builds packs on Linux, Windows and Apple Silicon macOS. Game dependencies download to ephemeral `.local/`; CI uploads only our original JAR and packs. Compilation is not Mac GPU gameplay testing. Publication checks allow Java only under our original `dev/horizonlab` namespace; review diffs as well.

[MIT](LICENSE) covers only our original tracked work. Dependencies retain their licenses. Mojang [removed obfuscation](https://www.minecraft.net/en-us/article/removing-obfuscation-in-java-edition) without changing [Minecraft's EULA](https://www.minecraft.net/en-us/eula). Decompiled game source remains local; published code implements independent tooling and instrumentation.
