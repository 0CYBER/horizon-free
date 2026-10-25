# Performance

Horizon is built to run Minecraft with less work: less memory, no pauses from a Java runtime, and a renderer that keeps the graphics card busy. This page says what Horizon does differently, and what has been measured. Where nothing has been published yet, it says so.

## What Horizon does differently

- **Native code.** Horizon is Rust, compiled for your system. There is no Java virtual machine, so no garbage-collection pauses and no warm-up while the code gets compiled as you play.
- **It ticks what vanilla ticks, and no more.** Mobs, minecarts and dropped items outside the simulation distance only check whether to despawn, as in vanilla. Before this was fixed, Horizon ticked seven times as many of them as vanilla at render distance 32.
- **Work off the main thread.** Chunks are generated, lit and meshed on worker threads, and unloaded chunks are written to disk in the background.
- **Fewer, bigger draw calls.** Terrain sections share one vertex arena and are drawn with one indirect multi-draw per pass. Mobs and block entities outside the view are skipped, and caves you cannot see into are culled.
- **Distant terrain.** The optional [LOD distance](/docs/settings#distant-terrain-lod) draws terrain up to 1024 chunks away without simulating it.
- **A server without Java.** `horizon-server` is one native program. See below.

## The client

The numbers below are Horizon before and after a round of performance work, on an RTX 4080, in a fresh creative world flown over a village, a forest, the ocean and a mountain (`tools/graphics_bench.py`). They compare Horizon with an earlier Horizon, not with vanilla.

- Render distance 8 over a village: **363 to 2,409 FPS**, and the slowest 1% of frames from 21.6 to 160.8 FPS.
- Render distance 16 over a village: **8 to 1,041 FPS**. In a forest: 28 to 316 FPS.
- Render distance 32 improved too (a village went from 4 to 48 FPS) but **still stutters in places**: the slow frames are game ticks while chunks load, not drawing.

The full tables, with the method and the Linux software-rendering run, are in [docs/graphics.md](https://github.com/ludovic111/horizon/blob/main/docs/graphics.md#results).

**Horizon against vanilla.** The benchmark exists: `tools/perf_bench.py` starts Horizon, vanilla 26.3 and vanilla with Sodium, Lithium, FerriteCore and ImmediatelyFast on the same machine, world, camera path and settings, and computes every number the same way. [docs/benchmark.md](https://github.com/ludovic111/horizon/blob/main/docs/benchmark.md) says how it keeps the comparison fair. Its results for the client, measured on the project's own machines, are on the [Benchmarks page](/benchmarks), with the places where Java is faster. It also estimates other hardware from them, as a range, and shows players' uploaded results once at least three installs share a graphics card and processor.

**Measure your own computer.** Horizon's half of that benchmark is built into the game. Close other heavy apps,
plug in a laptop, and run in a terminal:

```sh
horizon-launcher play --arg=--benchmark --arg=standard
```

Add `--arg=--render-distance --arg=16` to choose the render distance; your other settings are used as they are and
stored in the report. The game creates its own world, `horizon-benchmark` (it never touches your saves), flies over a
village, a forest, the ocean and a mountain, and quits by itself. `--arg=village` (or `forest`, `ocean`, `mountain`)
instead of `--arg=standard` flies one scene. For the run only, VSync is off and the frame rate unlimited; your options
are not changed.

The report is a `.json` file in the `benchmarks` folder of the game folder:

| System | Folder |
|---|---|
| Windows | `%APPDATA%\horizon\benchmarks` |
| macOS | `~/Library/Application Support/horizon/benchmarks` |
| Linux | `~/.local/share/horizon/benchmarks` |

It holds startup times, each scene's chunk load time, the frame rate with its 1% and 0.1% lows, memory, and your
machine (operating system, processor, memory, graphics card and driver). Its format is
[docs/benchmark-report.md](https://github.com/ludovic111/horizon/blob/main/docs/benchmark-report.md).

**Share it.** To add your computer to the [Benchmarks page](/benchmarks), either:

- upload it from the launcher: `horizon-launcher data benchmark <the report's path>` shows exactly what would be sent,
  without your file paths, and asks before sending. Or allow benchmark uploads once (`horizon-launcher data allow
  benchmarks`), and every later report is sent for you. See [Privacy](/privacy);
- or post the `.json` file in the Discord.

## Servers

The latest bot benchmark compares `horizon-server` with vanilla 26.3, Paper and vanilla on Fabric with Lithium. It ran on a 4-thread laptop CPU (an i7-7500U), with 1 to 50 bots walking, exploring and standing around spawn, on another machine. All four servers did the same work. In short:

- **Memory:** 0.7 GB with one player and 1.1 GB with 50, against 3.4 to 4.5 GB for the Java servers (each with a 4 GB heap). That is 3.8 to 4.8 times less.
- **Start:** ready in 8 to 9 seconds, against 15 for vanilla, 17 for Fabric and 19 to 22 for Paper.
- **CPU:** from 10 players up, the least of the four.
- **Average tick:** Paper's was shorter at every player count, and Fabric with Lithium's at 1 and 10 players. Horizon's was shorter than vanilla's, and than Fabric's from 25 players up.
- **Slow ticks:** from 10 players up, Horizon's slowest ticks were the shortest of the four.
- **50 players:** Horizon and Paper kept 20 TPS. Fabric with Lithium dropped to 17, and vanilla's watchdog stopped it.

For more than 50 active players, give it more cores. The full report is [benchmarks/server-four-way-2026-10-19](https://github.com/ludovic111/horizon/blob/main/benchmarks/server-four-way-2026-10-19/README.md).

## Where else to look

- [Benchmark report format](https://github.com/ludovic111/horizon/blob/main/docs/benchmark-report.md): what every benchmark writes.
- `HORIZON_SPIKES=MS` prints what made every frame slower than that many milliseconds slow; it is described in [docs/graphics.md](https://github.com/ludovic111/horizon/blob/main/docs/graphics.md#benchmark).
