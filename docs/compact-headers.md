# Compact object headers experiment

Java 25 HotSpot offers `-XX:+UseCompactObjectHeaders`. This changes object-header
representation without replacing game logic. Oracle documents the header format
and enable flag in its [Java 25 GC guide](https://docs.oracle.com/en/java/javase/25/gctuning/other-considerations.html).
Object alignment and fields determine whether a particular allocation gets smaller.
Native renderer buffers and GPU allocations are outside this optimization.

## Try on your Mac

Import the balanced pack normally. In Prism, edit the instance's Java settings
and add `-XX:+UseCompactObjectHeaders` to its JVM arguments, retaining any existing
arguments. Use Java 25 HotSpot matching your CPU architecture. Restart the game.
Remove the argument to return to the launcher's default, or use
`-XX:-UseCompactObjectHeaders` to explicitly test the disabled mode. Do not supply
both flags. Packs do not automatically enable this option.

For an isolated comparison, duplicate the same balanced instance and use explicit
off/on flags. Keep collector, heap, world copy, camera route and all video settings
matched. Alternate at least three warmed 120-second recordings. The 0.8 recorder
captures the actual `compact_object_headers` setting and `garbage_collectors` names;
check these in the offline viewer. Older reports show "not captured", rather than
an assumed default. Launcher and JVM defaults can differ.

Local demo and server tools accept `--compact-headers`. This enables the option
only in that process. The server still requires local EULA acceptance before it runs.

## Isolated allocation measurement

```sh
python3 tools/build_mod.py --benchmark-headers
```

The original benchmark uses the verified game/Distant Horizons publisher types
on its classpath, without redistributing them. It forks separate JVMs with explicit
off/on flags three times. Each type gets three discarded warm-up batches followed
by three measured batches of 100,000 objects. `ThreadMXBean` counts allocated bytes;
reference-array storage is allocated before measurement. Objects escape through a
volatile array, and their values are checked after measurement. The benchmark
asserts that the running VM reports the requested flag. Output includes Java/OS/CPU
architecture and goes to gitignored `dist/object-header-benchmark.txt`.

The local Linux x64 Java 25 measurements yielded:

| Type | Headers off, bytes/object | Headers on, bytes/object |
| --- | ---: | ---: |
| Minecraft `AABB` | 64 | 56 |
| Minecraft `Vec3` | 40 | 32 |
| Distant Horizons `DhChunkPos` | 24 | 24 |
| Boxed `Long`, outside cache | 24 | 16 |
| Plain `Object` | 16 | 8 |

These are allocations for specific object layouts, not a measured reduction in
whole-game retained heap, process RSS, FPS or server TPS. Changing headers can
also change runtime tradeoffs. Real workloads and the user's M2 Pro remain the
decision point. CI runs the probe on Windows, Linux and Apple Silicon macOS and
runs the metric tests with compact headers enabled as well as the default.

## Fresh video settings correction in 0.8

Minecraft 26.3 applies its selected graphics preset at startup. A fresh minimal
0.7 options file had the correct data-version marker but omitted an explicit
custom preset, allowing the default Fancy preset to override full-detail distance
to 16 chunks. Version 0.8 explicitly selects the custom preset so the pack's
12-chunk budget (8 in low-memory) is retained. This fixes configuration, without
reducing simulation distance or changing gameplay. Runtime reports capture the
effective settings; always compare those, not just pack labels.
