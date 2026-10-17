# Dedicated server against vanilla 26.3, with bots (2026-10-17)

`horizon-client --dedicated` (run by `horizon-server`) against the pinned vanilla 26.3 `server.jar`. Each
run uses `horizon-bots` and `tools/server_bench.py`, with 1, 10, 25, 50 and 100 bots, `walk` and then
`explore`. The two servers take turns at each bot count.

**Read this first: the machine was overloaded.** The Mac mini was shared with other agents' builds and
harnesses. Its load average was 55 to 141 on 14 threads, and the CPU was 77–89 % busy before either server
started (`load.txt`, sampled every 30 s). The 1- and 10-bot rows compare the servers fairly enough. Above
that, both servers collapsed: vanilla's watchdog killed it after a 60-second tick in 5 of the 10 large runs.
Treat those rows as "it falls over on a starved machine", not as each server's capacity.

## Setup

- **Machine:** Mac mini M4 Pro (14 threads, 64 GB), macOS 27.0.1.
- **Horizon:** release build of `main` at `123987c4` (`horizon-server`, `horizon-client`, `horizon-bots`).
  The `commit` field in the JSON is that build. `worktree_commit` is the checkout the script ran from.
- **Vanilla:** the pinned 26.3 `server.jar` on OpenJDK 25.0.4.1, with `-Xms4g -Xmx4g`.
- **Properties:**
  - `level-seed=12345`, view and simulation distance 10, creative (forced);
  - `online-mode=false`, `difficulty=easy`, `max-players=200`.
- **Each run:** a fresh server, bots joining one per second, a 60 s warm-up, then 180 s measured.
- **Command:**
  ```
  HORIZON_BENCH_BIN=<release binaries> python3 tools/server_bench.py --servers horizon,vanilla \
      --bots 1,10,25,50,100 --behaviour walk|explore --warmup 60 --seconds 180
  ```
- **Columns:**
  - **TPS and MSPT** come from `/tick query`.
  - **RSS** is the mean over the run. **CPU** is the process tree, as a percentage of one core.
  - **Chunks/s** is what the bots received.
  - **Load** is the machine's load average during that run.

## Results

### `walk`: bots walk around the spawn

| Bots | Server | TPS | MSPT avg | MSPT p99 | RSS | CPU | Chunks/s | Load |
|---:|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | Horizon | 20.0 | **4.9 ms** | 86 ms | **1.63 GB** | 18 % | 10.5 | 68 |
| 1 | vanilla | 20.0 | 13.4 ms | 84 ms | 3.47 GB | 26 % | 10.1 | 64 |
| 10 | Horizon | 20.0 | **11.5 ms** | 138 ms | **1.57 GB** | 38 % | 101.5 | 55 |
| 10 | vanilla | 20.0 | 21.2 ms | 112 ms | 3.46 GB | 49 % | 93.7 | 58 |
| 25 | Horizon | **0.9** | 1115 ms | 4513 ms | 1.61 GB | 45 % | 200.9 | 83 |
| 25 | vanilla | **20.0** | 38.8 ms | 250 ms | 3.58 GB | 69 % | 255.2 | 85 |
| 50 | Horizon | 0.46 | 2192 ms | 14169 ms | 1.71 GB | 24 % | 125.7 | 125 |
| 50 | vanilla | crashed | 60 s tick, watchdog | | | | | 141 |
| 100 | Horizon | 0.38 | 2638 ms | 17095 ms | 1.91 GB | 46 % | 222.0 | 109 |
| 100 | vanilla | crashed | 60 s tick, watchdog | | | | | 87 |

### `explore`: bots fly outwards in straight lines

| Bots | Server | TPS | MSPT avg | MSPT p99 | RSS | CPU | Chunks/s | Load |
|---:|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | Horizon | 20.0 | 11.6 ms | 123 ms | **1.69 GB** | 120 % | 44.5 | 95 |
| 1 | vanilla | 20.0 | 15.9 ms | 110 ms | 3.52 GB | 92 % | 32.3 | 110 |
| 10 | Horizon | 20.0 | 8.2 ms | 48 ms | 2.03 GB | 275 % | **10.8** | 80 |
| 10 | vanilla | 20.0 | 30.8 ms | 282 ms | 4.62 GB | 169 % | 41.4 | 66 |
| 25 | Horizon | 20.0 | 14.6 ms | 81 ms | 1.94 GB | 231 % | **0.0** | 76 |
| 25 | vanilla | crashed | 60 s tick, watchdog | | | | | 84 |
| 50 | Horizon | 13.9 | 72.2 ms | 285 ms | 2.69 GB | 163 % | **0.0** | 83 |
| 50 | vanilla | crashed | 60 s tick, watchdog | | | | | 98 |
| 100 | Horizon | 4.2 | 239 ms | 581 ms | 1.84 GB | 106 % | **0.0** | 122 |
| 100 | vanilla | crashed | 60 s tick, watchdog | | | | | 140 |

From 10 `explore` bots up, Horizon's ticks look short only because it does not do the work. Its bots
received few or no new chunks: generation and lighting fall behind fast fliers, and the hub sends only lit
chunks. Vanilla generated 4 times more terrain for 10 bots before it collapsed at 25. This build has no
movement checks (#586), so Horizon never pulls fast players back the way vanilla does.

## What it shows

- **1 to 10 players:** Horizon ticks in about half of vanilla's time with less than half its memory. That
  holds in both behaviours.
- **25 walking players: Horizon collapses and vanilla doesn't.** A 20-second `sample` of the main thread
  during a separate 25-bot `walk` run (MSPT 1464 ms) found the cause. 96 % of the server thread was inside
  `Hub::tick` → `ChunkEncoder::encode_in` (`crates/horizon-client/src/lan/chunks.rs`), encoding chunk
  packets for the bots:
  - `Paletted::from_values` re-palettes every section from the unpacked values;
  - `PalettedContainer::unpack_u16` is called, among other places, by `occupied()`, which unpacks the 24
    sections of each of the 9 neighbouring columns for every chunk sent;
  - `light_data` runs twice per chunk (`encode` computes it, and `encode_in` overwrites it);
  - `client_heightmaps` is computed again for each player.
  
  Every chunk is encoded again for every player who receives it. At about 200 chunks/s, that alone fills the
  tick.
- **Memory:** Horizon's RSS stayed at 1.6–2.7 GB in every run, against 3.5–4.6 GB for vanilla with its
  4 GB heap.

## Files

- `walk.json`, `explore.json`: the `horizon-server-benchmark` reports.
- `load.txt`: the machine's load average and CPU use every 30 s, with the share taken by agents.
