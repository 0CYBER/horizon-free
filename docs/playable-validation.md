# First playable release validation

Version 0.2.0 targets Minecraft 26.3 / Fabric 0.19.5 / Java 25. This is an initial playable Java optimization stack, not a completed renderer rewrite. Measurements here establish what was actually checked.

## Local checks

- Official client runs through a Wayland virtual screen in built-in limited demo mode, without Microsoft credentials. Normal Mac play uses licensed Prism sign-in.
- Balanced stack (Sodium, Lithium, FerriteCore, ImmediatelyFast, EntityCulling, Distant Horizons, Fabric API, Horizon Lab) reaches the menu, loads the demo world, logs in locally, renders terrain, and saves/quits.
- DH generates and accepts the current configuration schema: 128-chunk LOD radius, two background threads at 0.5 run-time ratio, priority 3. No old-version guessed config keys.
- A missing desktop AWT component in the local headless JRE initially caused a DH cloud-texture error. Installing the complete Java 25 runtime resolved it; this was an environment issue, not a packaged mod change.
- Native libraries are chosen by OS and architecture, including ARM64 selection when the official manifest has OS-only rules. Builds use portable `/` ZIP paths on Windows too.
- The official Prism 11.1.1 macOS ZIP was downloaded, SHA-256 verified, and its actual app layout checked. Its launcher executable is a universal Mach-O containing x86_64 and arm64. The starter does not redistribute it or disable macOS security checks.
- Dedicated-server optimized dependencies and original mod initialize with the real server game JAR; EULA-false startup exits as expected. No license acceptance is made on the user's behalf. Integrated-server gameplay is a separate client check.
- The original recorder loads with the vanilla renderer and F8 starts/stops it during actual demo gameplay. A persistent virtual keyboard is needed on this headless compositor: removing the last temporary keyboard otherwise removes window focus and auto-pauses the game. This is a test harness detail, not a change to pause behavior or focus exclusions.
- That runtime check wrote both raw CSV/JSON pairs, containing 69 actual frame intervals and 744 actual integrated-server tick durations. GPU/backend/driver and render settings were captured. The comparison tool successfully reads these real report files. This short, cold, software-rendered run validates capture/output only; it is not a performance baseline to compare with a real Mac.
- Build/metrics/pack tests pass on actual hosted Apple Silicon macOS 26, Windows x64 and Linux x64 runners. The Windows run exposed an existing decompilation-coverage path separator mismatch; paths are now normalized to match JAR SourceFile metadata.
- 17 offline Python tests cover corrupt/cached artifacts, server extraction/traversal rejection, decompilation inventory, publication exclusions, dependency closure, safe download paths, pack format, OS/ARM native selection, and comparison mismatches.
- Original Java metrics tests cover percentiles, the average of the slowest 1% (not the reciprocal of p99), stall boundaries, bounded samples/overflow, filename sanitization, exclusions and player counts.
- Existing three Rust differential tests, formatting and Clippy checks pass. Rust kernels remain unintegrated.

- The exact published balanced pack was then re-run through real demo world rendering, with its published original mod binary checked byte-for-byte against the runtime. F8 produced nonempty frame/tick JSON and matching raw CSV; each reported sample count and mean duration were checked against the CSV.

## Interpretation limits

The exact 0.11.0 JAR (SHA-256
`c2f59ca0b2f1e356f51b41cd0d9f1d98d6b86e240f5b5a1424f27de57f0e9c4a`)
ran `foreground-first` with an explicit G1 collector, compact headers and 3 GiB
heap. Its 120-second OpenGL world capture produced 640 frame intervals and
2,401 server ticks with matching CSV counts/means and 120 RSS samples each.
All four controls and the wakeable idle path were reported enabled, with adaptive
goal 120 FPS, uncapped rendering and actual distances 12/12. World saves, all
three DH database closures, worker shutdown and client quit completed. The offline
viewer displayed the actual combined state, goal/cap distinction and collector
accounting guidance. These software-rendered checks validate integration rather
than establishing M2 FPS, RAM, LOD-completion or real-GPU visual improvements.

The same exact JAR then ran `foreground-vulkan` with all controls enabled,
adaptive goal 120, uncapped rendering and distances 12/12. The actual Vulkan
backend and driver were captured. Its 120-second recording produced 618 frame
intervals and 2,413 server ticks with matching CSV counts/means and 120 RSS
samples each. This verifies the combination on Linux software Vulkan; it does
not validate MoltenVK, all distant-geometry visuals or GPU performance on the Mac.

The exact 0.10.0 JAR (SHA-256
`101790057c969333e2d9753640d4996649e8ced27e465897d8b6beab2474ed4c`)
ran the standalone dispatch-retry profile with 3 GiB heap and compact headers.
Its 120-second recording produced 568 frame intervals and 2,407 server ticks,
with matching CSV counts/means and 120/119 RSS samples. The actual frame report
retained custom graphics preset and 12/12 render/simulation distances, enabled
the dispatcher hook, and disabled adaptive/pacing/debounce/idle hooks. The world
saved, all three DH databases closed, worker threads stopped and the client quit.
The offline viewer read actual dispatch metadata, JVM settings, RSS and CSV.
Applied-hook fixtures verify exact task completion, independent-executor progress,
recovered slots and shutdown against the actual upstream rejection path in Linux
CI; Windows and ARM macOS compile the fixture. [Results and limits](dispatch-retry.md)
describe synthetic, caller-only failure-path measurements. An unchanged client
from the same saved world did not reproduce the earlier game allocation storm;
these variable captures do not prove a repeatable game-FPS or RAM improvement.

The exact 0.9.0 JAR (SHA-256
`2c5a731d4c93df427cc6054f120a52bf9456b3343b94ad9b79de1584b14d783f`)
ran the combined queue/adaptive/pacing profile with a 3 GiB heap and compact
headers enabled. Thread inspection reached the new wakeable idle park in all
three dimension queues. The 120-second capture produced 806 frame intervals and
2,411 server ticks, with matching CSV counts/means and 120 RSS samples each.
The frame report captures the idle hook as true and actual render/simulation
distances 12/12. The offline viewer reads the new flag, JVM settings and raw CSV;
general viewer checks also pass. World/database/worker shutdown completed cleanly.
Concurrency tests and the actual publisher-queue wake-latency probe pass on
Linux, Windows and hosted Apple Silicon macOS; [raw results and limits](debounce-queue.md)
keep idle wakeup separate from terrain completion and FPS. A 3 GiB heap passing
this short world test is not a recommendation for heavy or long-running worlds.

The exact 0.8.0 JAR (SHA-256
`b674be112cd148406b6b79c14f54b91a4da1ac0e03fe744aaee857c5a51eae8a`)
loaded the complete adaptive-queue stack with compact headers enabled. A fresh
options file retained custom graphics preset, render distance 12 and simulation
distance 12 in the actual frame report. The 120-second run produced 793 frame
intervals and 2,409 server-tick samples; CSV counts and mean durations match JSON.
Both reports captured compact headers as true, G1 collector names and 120 RSS
samples. The offline viewer displayed these actual settings, RSS and CSV charts;
general frame/tick, invalid-input, escaping and mobile checks also passed.
World/database shutdown completed cleanly. This software-rendered integration
test is not a matched RAM/FPS comparison. Cross-platform builds, native-reader
tests, metrics under compact headers, isolated allocation probes and 20 Python
tests passed on Linux, Windows and hosted Apple Silicon macOS 26. See
[object layouts and measurement limits](compact-headers.md).

Generated pack options now carry data version 5023, verified against `version.json`
inside the checksum-verified Minecraft 26.3 JAR. Without this marker, the game's
legacy-options migration reset the intended backend to Default. The fixed Vulkan
preference starts the actual Vulkan backend with the pinned Sodium/DH/ImmediatelyFast
stack, renders the demo world, and records 256 frame intervals / 2,372 server ticks
over 120 seconds with matching CSV/JSON and 119–121 RSS samples. The frame report
identifies Vulkan and its driver. DH data continues updating and saves/databases
close cleanly. With an unavailable Vulkan driver path confined to a separate test
process, the game reports backend creation failure and reaches the menu using its
OpenGL fallback. These checks do not establish Vulkan speed, distant-LOD visual
correctness, shader compatibility, or behavior on the user's Mac. Balanced remains
the OpenGL first test; Vulkan remains an optional independent comparison.

The 0.7.0 optional debounce queue hook initializes and renders with the combined
adaptive/pacing controls. The exact candidate JAR wrote 643 frame intervals and
2,408 server-tick samples over 120 seconds; CSV counts and means match the reports,
which capture all three controls as enabled and include resident-memory samples.
Thread inspection reached the injected debounce parking path, DH terrain data
continued updating, and worlds/databases/workers closed cleanly. The actual
publisher queue microbenchmark passes identity/priority/count checks on Linux,
Windows and Apple Silicon macOS; its raw results and timing overshoot are documented
in [the queue experiment](debounce-queue.md). These are separate from game-FPS,
block-update-latency or RAM-reduction claims. All fourteen pack overrides retain
simulation distance 12 and explicitly set the intended experimental flags.

The 0.6.0 resident-memory sampler ran in the actual adaptive-pacing demo world.
A 120-second recording produced 370 frame intervals and 2,407 server-tick samples;
both CSV counts and mean durations match their JSON summaries. Each report has
120 successful RSS samples, a peak of about 2,643 MiB, and a last-sample age under
800 ms. The runtime JAR matches the release candidate byte-for-byte. After capture,
the sampler thread waits without further scheduled reads. Native-read tests also
pass on hosted Windows x64, Linux x64 and Apple Silicon macOS 26; unsupported
reader behavior preserves timing samples and omits unavailable RSS fields.
These numbers validate reporting in this particular software-rendered workload,
not a RAM reduction or a prediction of the user's Mac memory use.

The 0.5.0 offline viewer passed Chromium checks using real frame/tick JSON and CSV
files, charts, exports, malformed inputs, kind mismatches, escaped profile labels,
and mobile layout. It is self-contained and has no network dependencies. Its game
runtime classes are identical to 0.4.0, apart from mod version metadata.

The 0.4.0 worker-pacing hook also renders the demo world with the complete mod stack.
Both pacing and adaptive controls are captured as enabled in the final build's
120-second frame report. It produced 359 frame intervals and 2,401 integrated-server
tick samples, verified against raw CSV counts and heap/resource fields. The DH database
received terrain-data updates during this run. The earlier hook build saved/quitted
and closed its databases/workers cleanly. Hosted executor benchmarks pass on Linux,
Windows and Apple Silicon macOS; [their results](worker-pacing.md) remain separate
from game-performance claims. The revised policy tests include 1 ms and 15 ms timer
overshoot and bounded credit after long stalls.

The 0.3.0 adaptive profile rendered the actual demo world and applied live DH duty-cycle
changes from 0.5 down to 0.05 under software-renderer frame pressure. Opening the pause
menu released the override and reported an effective ratio of 0.5 again. A separate
baseline instance starts without DH. API adapter tests cover suspension/resumption,
changed user ceilings, and yielding to an existing or changed override from another mod.
The real recorder wrote 193 frame intervals and 1,651 integrated-server tick samples,
with matching CSV counts, sampled heap peaks and process CPU data. Dedicated-server
0.3.0 dependencies initialize and exit with EULA false. No populated server result
is inferred. The updated offline suite has 19 Python tests, including upgrade backups
and refusing to replace an unrelated JAR with a matching filename.

A 60-second JFR recording identified foreground, server and DH worker activity. It
also demonstrated why allocation sampling must be corroborated: large sample weights
at timestamp boxing did not correspond to the largest per-thread allocation counter
deltas. Profiles stay private under `.local/`; neither sample weights nor this
software-renderer workload establish a real-hardware optimization result.

The local Linux host uses **Mesa llvmpipe software rendering**. It is useful for mod/mixin/startup/report validation, not for predicting M2 Pro, Windows GPU or Linux discrete-GPU performance. Warm-up/world loading on this host is slow. Demo credentials produce expected unavailable account-properties/Realms logs, and chat availability can be restricted; the independent F8 measurement shortcut avoids relying on chat.

CI compiles/builds on Linux, Windows and Apple Silicon macOS. Successful compilation is not graphics, authentication, UI or Mac-driver runtime validation. macOS 27 and the user's actual M2 Pro have not been tested here. Optional cinematic shaders and Vulkan are experiments; the OpenGL balanced pack is the first manual test.

The [core comparison](core-performance.md) records numerical observations for
matched software-rendered workloads. They do not establish a Mac or hardware-GPU
speedup, least-memory result, exhaustive vanilla equivalence, universal GPU
support, or a 50-active-player result. Worlds need copies, repeated matched routes
and real hardware measurements before decisions about deeper replacements. New
terrain generation and steady-state rendering are separate workloads.

## Release content

Packs embed only the original MIT Horizon Lab JAR and config. Mod/game/library downloads retain upstream licenses and are checked against pinned hashes. No game JAR, class, decompiled source, asset, world or credential is committed or uploaded in a starter. The narrow publication allowance is for original Java under `dev/horizonlab`, and the built JAR checks its emitted class namespace.
