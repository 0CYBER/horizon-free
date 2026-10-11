# Horizon

Horizon is a Rust rewrite of Minecraft Java **26.3** targeting one-to-one vanilla gameplay parity, alongside client performance research. Not affiliated with Mojang or Microsoft.

**The Rust game is now playable.** The [native client](docs/native-client.md) opens on a vanilla-style title screen with world creation and selection, and generates worlds with the Rust port of the vanilla overworld generator: noise terrain, aquifers, surface rules, carvers, and the placed features ported so far, all checked against vanilla. It streams and lights chunks on worker threads and renders every vanilla block model with smooth lighting, biome tints, the vanilla sky, sun, moon, stars and clouds. Gameplay so far:
- validated player physics, health and hunger;
- mining with vanilla tool rules and loot tables;
- placing, crafting, chests and furnaces;
- dropped items and particles;
- sounds;
- passive and hostile mobs with combat, including guardians, shulkers, illagers, ravagers, the wither and the ender dragon fight;
- villagers that take jobs, trade, restock, meet at the bell and sleep in beds;
- boats, minecarts, item frames, paintings and armor stands;
- shields, maps, books, fireworks, beacons, advancements and statistics;
- Anvil saves.

Parity is **not** complete. Still missing: the warden and the sculk family, the breeze, sniffer, creaking, happy ghast, llamas and a few other mobs, raids and pillager patrols, villager breeding and gossip, the loom, crafter and recipe book, scoreboards, subtitles, and spawned mob gear enchantments. Multiplayer joins vanilla servers but Horizon cannot host one. See [Rust rewrite status](docs/rust-parity.md).

### Java parity: 84%

Each subsystem scores 0 (absent), 0.25 (started, not playable), 0.5 (playable,
major vanilla features missing), 0.75 (playable, mostly complete) or 1 (complete
and verified against vanilla by differential tests). The percentage is the
average; `tests/test_parity.py` keeps it in sync with this table. Last audited
2026-10-10 against the code, not the docs. The harnesses that failed in the
previous audit (block updates, random ticks, redstone, command parsing, menu
clicks, whole chunks, entity models and item models) now pass; the save
round-trip harness has not been rerun.

| Subsystem | Score | Main gaps |
| --- | ---: | --- |
| Window, input and game loop | 0.75 | Game loop and options match vanilla; camera, auto-jump, toggle sprint/sneak and the frame limiter not compared |
| Assets and resources | 0.75 | Item models match vanilla over 196,290 stacks, but the client does not draw items through that pipeline yet; no pack screen |
| Network protocol and multiplayer | 0.75 | Opens worlds to LAN, but hosting is not tested against a vanilla client; no dedicated server mode |
| Chunk storage and palettes | 1 | None: client and saves share paletted containers, identical to vanilla over 266 scenarios |
| Section meshing | 1 | None: 112 scenes and over a million quads identical to vanilla |
| Rendering | 0.75 | Entity models match over 150 layers; block entity renderers and HUD overlays not compared |
| Lighting | 1 | None: saved and relit light matches vanilla, slabs and stairs included |
| World generation | 1 | None: 2,352 whole chunks over 225 structure sites, in all three dimensions, identical to vanilla |
| Region files and saves | 0.75 | Save round-trip harness fails; older DataVersions are not upgraded |
| Entities and physics | 0.75 | Attributes and dimensions match; AI, loot and spawn rules not compared; drowned, villagers, traders and the wither unverified |
| Interaction and inventory | 1 | None: menu clicks match over 6,000 click sequences |
| Menus, HUD and chat | 0.75 | Commands parse as vanilla over 39,685 inputs, but /item modify, /spectate, /waypoint and /dialog do not run; no tab list or world border rendering |
| Audio | 0.75 | Sound engine, music and ambience not compared with vanilla |
| Block behaviours and ticking | 0.75 | Block updates, random ticks and redstone match vanilla; dispensers lack several behaviours |

### Next steps

1. Rerun the save round-trip harness and upgrade saves from older DataVersions.
2. Draw every item through the item model pipeline, and add the resource pack screen.
3. An audio harness: sound engine attenuation, weighted sound events, music and ambience.
4. Mob AI, loot and spawn-rule harnesses, and an oracle for drowned, villagers, traders and the wither.
5. The missing commands and dispenser behaviours, the tab list and world border rendering.
6. Test LAN hosting with a vanilla client, and add a dedicated server mode.

```sh
python3 tools/prepare_native_client.py --prepare   # needs JDK 25; extracts private vanilla resources
python3 tools/fetch_sounds.py                      # optional: vanilla sounds
cargo run --release -p horizon-client
```

**Target machine:** the Rust rewrite targets one machine, **ludodesktop**: Ubuntu 26.04, Intel i7-13700KF, NVIDIA RTX 4080, GNOME on Wayland. The final deliverable is a playable native Linux build validated there. macOS (including the M2 Pro Mac) and Windows are out of scope. Track implementation and acceptance criteria in [Rust rewrite status](docs/rust-parity.md).

**Java performance packs (no longer developed):** the earlier track of pinned Java performance packs, distant terrain, an original frame/tick recorder and client/server starter downloads. No release downloads are published; build the packs and starters locally with `python3 tools/build_release.py` (see [Build and run](#build-and-run)). They are no longer a development target.

## Java packs: Mac starter (out of scope)

Build **horizon-mac-starter-0.21.0.zip** with `python3 tools/build_release.py` (it lands in `dist/`), extract it and open `Mac-Start.command`. It verifies and installs official universal Prism Launcher in `~/Applications` if needed, then opens the balanced pack import. Sign into Microsoft **inside Prism yourself**. This project needs no credentials.

Use native **Java 25 ARM64**, maximum heap **6144 MiB**, minimum **512 MiB** on your 16 GB M2 Pro. Prism can download Java automatically. Start at 1280×800 or 1920×1200; full Retina resolution is a larger GPU workload. [START-HERE.txt](packs/START-HERE.txt) has details. The same `.mrpack` files import into Prism on Windows and Linux.

If Prism is already installed, follow the [Mac import and first comparison guide](docs/mac-first-test.md).

Core and balanced also pass [macOS title/demo-world rendering probes](docs/client-graphics-smoke.md)
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
[Core comparison protocol and selected observations](docs/core-performance.md)
include three repeated matched-view software-rendered pairs with 14–21% higher
FPS in this workload; M2 performance remains unmeasured.

[Heap budget experiment](docs/heap-budget.md): three pairs with 3 GiB versus
1.5 GiB maximum heap show variable RAM savings and GC/frame-time tradeoffs;
playable defaults remain unchanged.

[Matched graphics API comparison](docs/graphics-api-comparison.md): Vulkan adds
about 1.5% average FPS but about 12% CPU in three Linux software-rendered pairs;
backend defaults remain unchanged and physical Mac performance is unmeasured.

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

The instance's `horizon-benchmarks/` folder receives JSON/CSV frame and integrated-server tick timings: average FPS, slowest-1%-average FPS, p95/p99 durations, stalls, JVM heap/GC, sampled process RSS when available, GPU/backend/driver and render settings. Frames include presentation/limiting waits and exclude menus/pauses/unfocused time. Tick `meanRate` is execution capacity, **not TPS**; observed tick counts are separate. Version 0.20 [captures sampled tick-manager state](docs/tick-state.md), including restored freeze/step/sprint/rate changes; this does not measure completed world work. Resident process memory is sampled on a shared background thread only during recordings; it includes native resident pages, while heap measures JVM objects. Neither is a complete measurement of unified GPU memory. [Memory measurement details](docs/process-memory.md).

```sh
python3 tools/compare_benchmarks.py baseline-frames.json optimized-frames.json --strict
```

Open `Compare-Benchmarks.html` from the starter ZIP or [the source file](tools/Compare-Benchmarks.html) in your browser to compare without Python. Select each JSON and optionally its matching CSV. It exports summary JSON, flags settings and duration differences, and charts frames by recorded active time or ticks by sample number. It is self-contained and makes no network requests.

Version 0.18 adds an isolated [terrain quad reuse experiment](docs/quad-reuse.md).
It reuses a temporary quad discarded by successful premerge when the existing
pool would otherwise allocate. Generated geometry and thread ownership checks
pass; allocation results are not whole-game RAM or FPS evidence. It remains off
in combined profiles.

Version 0.17 also records loaded mod versions and the Java vendor, VM and runtime
build in client and server reports. Both comparison tools flag software changes
as explicit experiment variables; older reports remain usable with an unknown
software warning. The viewer can expand the complete loaded mod list.
[Live software identity check](benchmarks/software-identity-2026-10-04/live-observations.json)
records the metadata and file-count validation; its short capture is not a
performance comparison.


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

Gitignored `dist/` contains twenty packs, our original instrumentation JAR, client/server starter ZIPs and SHA256SUMS. [mods.lock.json](packs/mods.lock.json) pins mod versions, exact dependency closure, SHA-1/SHA-512 and source/license; [fabric.lock.json](packs/fabric.lock.json) pins loader/library SHA-256. Packs use the [Modrinth format](https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack). They embed only original project code; third-party mods, game and assets download from their publishers.

The server starter uses Lithium/FerriteCore and `/horizonserver bench <label> <seconds>`. [Server instructions](docs/server-quickstart.txt) cover setup and a baseline. Files are prepared before you accept Minecraft's EULA locally. A 50-active-player workload remains to be measured.

Local smoke testing uses the game's built-in limited demo; normal play uses Prism's licensed account authentication:

```sh
python3 tools/build_pack.py --profile m2-balanced --instance .local/runtime/m2-balanced
vscreen run python3 tools/runtime.py demo --instance .local/runtime/m2-balanced
```

The standalone demo launcher applies the pinned official manifest's conditional
JVM flags, including macOS `-XstartOnFirstThread` and the Windows heap-dump-path
option. Prism applies its own official manifest. Server JVM policy is separate;
this launcher correction is not a measured graphics performance improvement.

## Local game inspection

Game JARs, decompiled Java, libraries, inventories, worlds and logs live **inside this checkout** under gitignored **`.local/`**. No game code is in Git history or release downloads. Fresh clones regenerate the inspection workspace from official downloads; Microsoft credentials are not needed for this step.

```sh
git clone https://github.com/ludovic111/horizon.git
cd horizon
git config core.hooksPath .githooks
python3 tools/lab.py prepare
python3 tools/lab.py decompile
python3 tools/lab.py status
```

`research-lock.json` pins the official manifest and Vineflower. The inner server JAR is extracted and checked; client/server libraries provide decompiler context. Output: `.local/minecraft/26.3/sources/{client,server}/` and coverage/inventory reports. Current inventories cover 11,383 client classes and 7,762 server classes, producing 7,301/5,037 Java files. See [decompilation status](docs/decompilation-status.md).

Class-file SourceFile metadata covers inner classes, which often share a source file. Coverage does not prove semantic equivalence or recompilability; original bytecode remains the reference. Completed source directories are not overwritten automatically. Older obfuscated releases require a separate remapping stage.

## Original Rust research

The dependency-free `horizon-core` crate starts the Rust simulation foundation:
integer block/chunk/section positions, legacy and xoroshiro seeded random streams and positional factories,
padded palette-index storage, scheduled block/fluid tick queues, and double-precision
bounding-box and voxel-grid collision geometry. Noise samplers, octave stacks and
bulk density kernels provide the next world-generation foundation. Compiled
density samplers add gradients, shifted and blended terrain noise, distance
metrics and arithmetic evaluation.
Differential runners compare selected
operations with the checksum-pinned vanilla client and server bytecode.
[Implemented scope, validation, and remaining rewrite work](docs/rust-parity.md).

```sh
python3 tools/test_rust_parity.py --prepare
python3 tools/test_world_parity.py
python3 tools/test_tick_parity.py
python3 tools/test_geometry_parity.py
python3 tools/test_collision_parity.py
python3 tools/test_noise_parity.py
python3 tools/test_density_parity.py
```

The core corpus checks 1,634,520 output lines per reference JAR, including
signed boundaries, RNG rejection/state consumption and raw storage words.
Passing it does not establish full gameplay parity or an FPS improvement.
The separate [noise suite](docs/noise-generation.md) checks 26.3 Perlin/simplex,
derivatives, density-volume accumulation, stack composition, normal-noise
normalization and modern/legacy initialization against both reference JARs.
The [density suite](docs/density-functions.md) checks compiled scalar and bulk
evaluation, including nested samplers, their different arithmetic paths, and
selected interval operations used for graph range analysis.

`horizon-world` adds NBT, four region compression codecs, `.mca`/`.mcc`
reading and export, saved palettes, a version-checked block-state registry,
and packed resident block sections. The world-data suite checks Rust output
with the actual vanilla NBT, region and block-state decoders on both JARs.
It covers all 35,723 exported state IDs and a 100,000-edit section trace.
[World-data commands and validation limits](docs/world-data.md).

Scheduled ticks now support vanilla ordering, duplicate suppression, save/reload,
chunk eligibility and callback mutations. The tick suite compares 97,776 output
lines per reference JAR and checks saved block/fluid lists through vanilla codecs
and region files. [Scheduler behavior and remaining integration](docs/scheduled-ticks.md).

Voxel shapes preserve vanilla collision tolerances, greedy box ordering, ray faces,
inside hits, and cube-versus-array cell lookup. A cached collision registry connects
imported shapes to resident block sections while checking exact state identities.
Movement helpers resolve supplied colliders and step-up candidates, including
landing adjustment and the second collider query. Movement-path block visits
preserve vanilla ordering, step indices and packed-position deduplication.
Uncached states require block behavior; the entity-context overload also needs
separate implementation. [Geometry and collision scope](docs/collision-geometry.md).

```sh
cargo test --workspace
cargo run --release --bin voxel-bench -- 100 > .local/voxel-benchmark.csv
```

The dependency-free occupancy experiment compares scalar and bitset exposed-face counting for synthetic 16³ sections. It has no real mesh, material/AO/light work, neighboring-section context, JNI bridge or game integration. Speedups are kernel-only, not Minecraft FPS. [Research plan](docs/optimization-plan.md) describes profiling, improved Java baselines, native integration and behavior checks before expanding scope.

Public branding and new mod downloads use **Horizon**. The existing Fabric mod
ID and `dev.horizonlab` namespace remain stable for installed instances and
measurement compatibility. The installer backs up both old `horizon-lab-*.jar`
and new `horizon-*.jar` versions before installing an update.

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

CI checks offline tools/Rust and compiles the original mod/builds packs on Linux and Apple Silicon macOS. Game dependencies download to ephemeral `.local/`; CI uploads only our original JAR and packs. Compilation is not Mac GPU gameplay testing. Publication checks allow Java only under our original `dev/horizonlab` namespace; review diffs as well.

[MIT](LICENSE) covers only our original tracked work. Dependencies retain their licenses. Mojang [removed obfuscation](https://www.minecraft.net/en-us/article/removing-obfuscation-in-java-edition) without changing [Minecraft's EULA](https://www.minecraft.net/en-us/eula). Decompiled game source remains local; published code implements independent tooling and instrumentation.

Integrated-server reports record the shared graphics backend at recording start; see [the context and comparison limits](docs/server-graphics-context.md).
