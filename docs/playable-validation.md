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
- 17 offline Python tests cover corrupt/cached artifacts, server extraction/traversal rejection, decompilation inventory, publication exclusions, dependency closure, safe download paths, pack format, OS/ARM native selection, and comparison mismatches.
- Original Java metrics tests cover percentiles, the average of the slowest 1% (not the reciprocal of p99), stall boundaries, bounded samples/overflow, filename sanitization, exclusions and player counts.
- Existing three Rust differential tests, formatting and Clippy checks pass. Rust kernels remain unintegrated.

## Interpretation limits

The local Linux host uses **Mesa llvmpipe software rendering**. It is useful for mod/mixin/startup/report validation, not for predicting M2 Pro, Windows GPU or Linux discrete-GPU performance. Warm-up/world loading on this host is slow. Demo credentials produce expected unavailable account-properties/Realms logs, and chat restrictions prevent chat commands; the independent F8 measurement shortcut supports this case.

CI compiles/builds on Linux, Windows and Apple Silicon macOS. Successful compilation is not graphics, authentication, UI or Mac-driver runtime validation. macOS 27 and the user's actual M2 Pro have not been tested here. Optional cinematic shaders and Vulkan are experiments; the OpenGL balanced pack is the first manual test.

No numerical game speedup, least-memory claim, exhaustive vanilla equivalence, universal GPU support, or 50-active-player result is claimed. Worlds need copies, repeated matched routes and real hardware measurements before decisions about deeper replacements. New terrain generation and steady-state rendering are separate workloads.

## Release content

Packs embed only the original MIT Horizon Lab JAR and config. Mod/game/library downloads retain upstream licenses and are checked against pinned hashes. No game JAR, class, decompiled source, asset, world or credential is committed or uploaded in a starter. The narrow publication allowance is for original Java under `dev/horizonlab`, and the built JAR checks its emitted class namespace.
