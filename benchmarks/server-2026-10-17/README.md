# Dedicated server against vanilla 26.3, with bots (2026-10-17)

`horizon-client --dedicated` (run by `horizon-server`) against the pinned vanilla 26.3 `server.jar`, on `main`
at `7bbbf6e9` (in release v2026.10.17.17). That build has the movement checks and chunk pacing of #586, the
equal-work method of #670, and the server performance PRs #701–#707 and #713.

This replaces the data first published here, which came from a Mac mini overloaded by other work (load 55–141 on
14 threads). An earlier run of this method, on `main` at `002f14ce`, is kept in `earlier-002f14ce/` and compared
at the end.

## Setup

- **Server machine:** ThinkPad, Intel i7-7500U (2 cores, 4 threads), 32 GB, NixOS. Nothing else heavy ran on it:
  its load average was 0.4 when idle. Both servers ran under `nice -n 19`.
- **Bots:** `horizon-bots` on another machine (Mac mini M4 Pro), over the tailnet. That machine was shared with
  other work (load 36–170 on 14 threads, mean 78). The bots need little CPU, but a busy bot host can answer later.
  Both servers saw the same conditions, since they took turns at each bot count.
- **Horizon:** release build of `7bbbf6e9` (`horizon-server`, `horizon-client`).
- **Vanilla:** the pinned 26.3 `server.jar` on OpenJDK 25.0.4.1, with `-Xms4g -Xmx4g`.
- **Properties:**
  - `level-seed=12345`, view and simulation distance 10, creative (forced);
  - `online-mode=false`, `difficulty=easy`, `max-players=200`;
  - default game rules on both (the locator bar is on).
- **Each run:** a fresh server, bots joining one per second, a 60 s warm-up, then 180 s measured.
- **Commands** (on the server machine):
  ```
  python3 tools/server_bench.py --servers horizon,vanilla --nice 19 --warmup 60 --seconds 180 \
      --bots-via "ssh <bot host>" --bots-bin <horizon-bots> --connect <server address> \
      --bots 1,10,25,50,100 --behaviour walk,explore
  python3 tools/server_bench.py ...same... --bots 1,10,25,50 --behaviour explore --explore-speed 0.8
  ```
- **Columns:**
  - **MSPT** (mean) and **P95** come from `/tick query`, in ms. **TPS** is the tick rate.
  - **RSS** is the mean over the run. **CPU** is the whole process tree, as a percentage of one core.
  - **Chunks/s** is what the bots received. **New chunks/s** counts each chunk once per bot.
  - **Pulled back** counts the times the server moved a bot back (vanilla's "moved too quickly" and similar).
  - **Load** is the mean load average during that run: the server machine (which includes the server's own
    threads) and the bot host.

## Results

### Equal work: bots explore at the same pace (`--explore-speed 0.8`)

Each bot flies outwards at 0.8 blocks a second, which both servers keep up with. They send the same chunks, so
the tick times compare the same work.

| Bots | Server | MSPT | P95 | TPS | RSS | CPU | Chunks/s | New chunks/s | Pulled back | Load (server / bots) |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | Horizon | 3.16 | 5.78 | 20.0 | 0.76 GB | 22 % | 2 | 1.27 | 0 | 2.8 / 63 |
| 1 | Vanilla | 16.82 | 27.3 | 20.0 | 3.37 GB | 69 % | 2 | 1.35 | 0 | 1.8 / 94 |
| 10 | Horizon | 9.83 | 19.01 | 20.0 | 0.86 GB | 77 % | 14 | 12.31 | 1051 | 2.2 / 74 |
| 10 | Vanilla | 24.38 | 39.58 | 20.0 | 3.57 GB | 132 % | 14 | 12.32 | 1027 | 2.5 / 64 |
| 25 | Horizon | 16.87 | 30.88 | 20.0 | 0.89 GB | 111 % | 34 | 29.92 | 3770 | 5.1 / 51 |
| 25 | Vanilla | 30.29 | 43.47 | 20.0 | 3.61 GB | 176 % | 33 | 29.29 | 5139 | 5.2 / 55 |
| 50 | Horizon | 23.57 | 47.53 | 20.0 | 0.92 GB | 127 % | 67 | 59.42 | 7434 | 8.5 / 64 |
| 50 | Vanilla | 42.44 | 54.84 | 20.0 | 3.67 GB | 201 % | 65 | 58.11 | 10604 | 5.1 / 59 |

### `walk`: bots walk around the spawn

| Bots | Server | MSPT | P95 | TPS | RSS | CPU | Chunks/s | New chunks/s | Pulled back | Load (server / bots) |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | Horizon | 4.01 | 8.19 | 20.0 | 0.78 GB | 59 % | 10 | 0.43 | 0 | 1.4 / 70 |
| 1 | Vanilla | 18.22 | 29.34 | 20.0 | 3.43 GB | 69 % | 10 | 0.47 | 0 | 1.9 / 63 |
| 10 | Horizon | 8.79 | 17.35 | 20.0 | 0.78 GB | 119 % | 95 | 3.58 | 0 | 2.9 / 69 |
| 10 | Vanilla | 18.67 | 38.26 | 20.0 | 3.50 GB | 101 % | 101 | 4.38 | 5 | 3.2 / 59 |
| 25 | Horizon | 15.01 | 31.12 | 20.0 | 0.80 GB | 161 % | 234 | 8.49 | 0 | 6.3 / 43 |
| 25 | Vanilla | 18.78 | 28.16 | 20.0 | 3.52 GB | 155 % | 252 | 11.89 | 10 | 6.7 / 57 |
| 50 | Horizon | 22.72 | 60.19 | 20.0 | 0.81 GB | 180 % | 465 | 16.94 | 0 | 14.7 / 50 |
| 50 | Vanilla | 34.3 | 55.55 | 20.0 | 3.58 GB | 267 % | 497 | 26.74 | 39 | 9.5 / 56 |
| 100 | Horizon | 63.33 | 125.04 | 15.79 | 0.87 GB | 246 % | 678 | 31.8 | 0 | 32.9 / 95 |
| 100 | Vanilla | stopped by its watchdog (a 60 s tick) | | | | | | | | 15.1 / 120 |

Both servers sent each walking bot the same number of distinct chunks over the whole run (680–690). Horizon's
"new chunks/s" inside the measured window is lower, so compare the totals, not that column.

### `explore` at full speed: unequal work

Bots fly outwards as fast as chunks arrive (`horizon-bots`' default, up to 22 blocks a second). Both servers now pull them back ("moved too quickly"), but Horizon sends
more new terrain, so this pass doesn't compare equal work. The server-machine load of Horizon's 1-bot run is
estimated from the time between the runs around it: its log was overwritten before it was read.

| Bots | Server | MSPT | P95 | TPS | RSS | CPU | Chunks/s | New chunks/s | Pulled back | Load (server / bots) |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | Horizon | 9.73 | 20.99 | 20.0 | 0.84 GB | 330 % | 41 | 35.96 | 0 | 4.7 / 129 |
| 1 | Vanilla | 9.71 | 20.37 | 20.0 | 3.61 GB | 247 % | 35 | 33.61 | 0 | 4.3 / 156 |
| 10 | Horizon | 21.27 | 39.15 | 20.0 | 1.18 GB | 322 % | 46 | 46.11 | 9856 | 5.0 / 114 |
| 10 | Vanilla | 16.94 | 30.28 | 20.0 | 3.95 GB | 254 % | 25 | 25.01 | 11123 | 5.5 / 89 |
| 25 | Horizon | 25.76 | 45.01 | 20.0 | 1.17 GB | 314 % | 61 | 60.7 | 31264 | 6.6 / 82 |
| 25 | Vanilla | 24.93 | 37.09 | 20.0 | 4.04 GB | 305 % | 38 | 37.84 | 33467 | 6.1 / 91 |
| 50 | Horizon | 39.31 | 65.33 | 20.0 | 1.25 GB | 325 % | 115 | 112.25 | 64163 | 6.9 / 95 |
| 50 | Vanilla | stopped by its watchdog (a 60 s tick) | | | | | | | | 7.7 / 86 |
| 100 | Horizon | 52.7 | 85.73 | 18.97 | 1.36 GB | 328 % | 154 | 151.83 | 116161 | 13.4 / 79 |
| 100 | Vanilla | stopped by its watchdog (a 60 s tick) | | | | | | | | 10.0 / 72 |

Vanilla's watchdog stopped it after a 60-second tick in three runs (walk 100, explore 50 and 100). Each time its
server thread was in entity tracking (`ChunkMap$TrackedEntity.updatePlayer`).

## What it shows

- **Same work, 1 to 50 players:** Horizon's mean tick is 44–81 % shorter than vanilla's, its 95th percentile is
  lower, and its process uses 32–63 % of vanilla's CPU.
- **Walking, 1 to 50 players:** Horizon's mean tick is shorter at every count (4.0 against 18.2 ms with 1 bot,
  22.7 against 34.3 ms with 50).
- **Where vanilla wins:**
  - With 25 and 50 walking bots, vanilla's 95th percentile is lower (28.2 against 31.1 ms, 55.6 against
    60.2 ms). Horizon's slow ticks are spikier.
  - In the full-speed explore pass, vanilla's mean tick is shorter with 10 and 25 bots (16.9 against 21.3 ms,
    24.9 against 25.8 ms), while Horizon sends 1.6–1.8 times as many new chunks.
  - With 50 walking bots, Horizon makes the server machine busier (load 14.7 against 9.5). Its connection
    writer threads compress every packet (below).
- **100 players:** Horizon keeps 15.8 TPS walking and 19.0 TPS exploring at full speed. Vanilla's watchdog
  stopped it in both.
- **Memory:** Horizon used 0.76–0.92 GB in every walk and equal-work run, and 1.36 GB at most when 100 bots
  explored at full speed. Vanilla used 3.4–4.0 GB with its 4 GB heap.
- **Start:** Horizon's fresh world was ready in 9.2–9.9 s, vanilla's in 14.9–17.2 s.

## Where Horizon's time goes (50 walking bots)

`perf record -e cycles:u --call-graph lbr` on the engine for 30 s, during a separate 50-bot `walk` run on the same
build and machine (MSPT 21.7 ms, 178 % CPU). Shares of all the engine's CPU samples:

| Thread | Share | Mostly |
|---|---:|---|
| Connection writers (`horizon-lan-out`) | 55 % | 53 % in `miniz_oxide` deflate, from `horizon_protocol::frame::encode` |
| World generation workers | 33 % | noise, surface rules, aquifers |
| Server thread (the tick) | 12 % | see below |

Inside the tick:
- the locator bar (`Waypoints::tick`): a third of the tick;
- `Hub::tick`: a quarter;
- block entities, creatures, chunk encoding (`ChunkEncoder::encode_in`) and `Streaming::update`: 7–10 % each;
- SipHash of `u128` keys (the default `HashMap` hasher): about 10 % of the tick.

**The next biggest cost is compressing packets.** Every chunk packet is compressed again for each player who gets
it, with `flate2`'s default `miniz_oxide` backend at level 6. On this 4-thread CPU that competes with the tick
and with world generation. Two fixes, which can be combined:
- the `zlib-rs` backend for `flate2`: 2.2–10 times faster than `miniz_oxide` at level 6 in a quick test, with
  output about 3 % larger;
- compressing a chunk packet once and sending the same bytes to every player who needs it.

## Before and after the performance PRs

The earlier run, on `main` at `002f14ce` with #670's bench changes (build `a9ee56a8`), before #586, #701–#707 and
#713. Same machines and settings. MSPT mean / P95, in ms:

Walk:

| Bots | Horizon before | Horizon now | Vanilla before | Vanilla now |
|---:|---|---|---|---|
| 1 | 5.77 / 11.04 | 4.01 / 8.19 | 18.84 / 29.55 | 18.22 / 29.34 |
| 10 | 16.23 / 29.32 | 8.79 / 17.35 | 18.02 / 37.62 | 18.67 / 38.26 |
| 25 | 23.98 / 49.18 | 15.01 / 31.12 | 19.19 / 33.84 | 18.78 / 28.16 |
| 50 | 35.07 / 73.24 | 22.72 / 60.19 | 35.23 / 60.87 | 34.3 / 55.55 |
| 100 | 104.8 / 158.8 | 63.33 / 125.04 | watchdog | watchdog |

Equal work:

| Bots | Horizon before | Horizon now | Vanilla before | Vanilla now |
|---:|---|---|---|---|
| 1 | 4.78 / 7.3 | 3.16 / 5.78 | 16.63 / 26.23 | 16.82 / 27.3 |
| 10 | 15.19 / 25.66 | 9.83 / 19.01 | 24.65 / 40.81 | 24.38 / 39.58 |
| 25 | 23.37 / 42.59 | 16.87 / 30.88 | 30.28 / 44.81 | 30.29 / 43.47 |
| 50 | 30.59 / 67.94 | 23.57 / 47.53 | 43.52 / 56.18 | 42.44 / 54.84 |

Vanilla's numbers repeat within a few percent, so the runs are comparable. In the earlier run, 25 walking bots
were faster on vanilla (19.2 against 24.0 ms). A profile then put a third of the tick in advancement updates,
which #701 fixed.

## Files

- `walk-explore.json`: the `walk` and full-speed `explore` passes. `explore-equal-rate.json`: the equal-work pass.
- `load.txt`: both machines' load averages every 30 s during the runs.
- `earlier-002f14ce/`: the same files for the earlier run.
- The server's tailnet address is replaced by `<server address>` in the JSON.
