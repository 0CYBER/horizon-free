# Horizon's server against vanilla 26.3, equal work (2026-10-17)

The same bots on Horizon's dedicated server and on vanilla 26.3's, one server at a time, on the same
machine, with the work each server does made the same before the numbers are compared. This replaces the
first run in docs/server.md section 9, where the work wasn't equal.

## What was made equal

The first run (PR #556) had three differences that made the comparison unfair. Each is fixed in the code
that ran here:

- **Refused moves.** Vanilla pulls a player back when a move is too fast ("moved too quickly") or ends
  inside a block ("moved wrongly"). Horizon now runs both checks the way vanilla does (PR #586), so a bot
  that flies too fast is pulled back on both servers.
- **Chunk sending.** Vanilla paces chunks by the client's acknowledgements (PlayerChunkSender: one
  unacknowledged batch at first, then ten, each at most the client's desired chunks per tick). Horizon used
  to send twice as many per batch; it now paces them the same way (PR #586), and the bots acknowledge
  batches and ask for a rate the way vanilla's client does, so both servers send at the pace the client
  asks for. Chunks sent again while the bot still held
  them (`resent`) are counted: 0 on both servers in every run.
- **Movement gated on chunks.** A bot doesn't move while the chunk it stands in hasn't arrived (vanilla's
  client waits the same way), so a slower server isn't asked to teleport bots into terrain it hasn't sent.

One difference isn't in Horizon yet: vanilla 26.3's locator bar (waypoints), which costs vanilla O(players²)
work per move. Every vanilla run here sets `gamerule locator_bar false`, so neither server does it.

## Method

`tools/server_bench.py` runs each configuration on a fresh server:

- the same `server.properties` on both (seed 12345, view and simulation distance 10, offline, creative,
  difficulty easy, `sync-chunk-writes=true`), and `gamerule respawn_radius 0`;
- the server waits for `Done`, the bots join one a second, a 60-second warm-up, then 300 seconds of
  measurement;
- MSPT (average, P50, P95, P99) comes from each server's own `/tick query`, asked every 5 seconds on the
  console, so both measure themselves the same way (vanilla's ServerTickRateManager numbers, which Horizon
  reproduces); TPS is 20 while the average fits in 50 ms, else 1000 / MSPT;
- RSS and CPU are the server's whole process tree, once a second (100 % is one core);
- chunk counts come from the bots: chunks received per second, chunks no bot had before (new terrain), and
  pull-backs (position packets that move a bot back).

The bots are `horizon-bots` (creative, flying 24 blocks above spawn, 20 moves a second):

- `walk`: flies to random points within 48 blocks of spawn at sprint speed (5.6 blocks a second);
- `explore`: flies straight outwards, by default at about 22 blocks a second, as fast as the server sends
  chunks.

Machine: the ThinkPad (Intel Core i7-7500U, 2 cores / 4 threads, 32 GB, NixOS), servers under `nice -n 19`
because it also runs other services; vanilla on OpenJDK 25.0.4.1 with a 4 GB heap (`-Xms4g -Xmx4g`). The
bots ran on the Mac mini and reached the ThinkPad over the tailnet, so they took none of the server's CPU.

What ran: Horizon from `origin/main` at bf851bfe with #586 (move checks and chunk pacing), the bots and
`server_bench.py` from this PR, #623 (sections encoded once, its first commit) and #649 (a tick's packets
handed to each connection together). The ThinkPad copy wasn't a git checkout, so the JSON reports have an
empty `commit`. Vanilla: the pinned 26.3 server jar.

## Results

MSPT is the average milliseconds per tick (50 is the limit for 20 TPS); P95 is the 95th percentile. CPU is
the whole process tree. "New chunks/s" are chunks no bot had before.

### Walk (the same work on both servers)

| Bots | Server | MSPT | P95 | TPS | RSS | CPU | Chunks/s | New chunks/s |
|---:|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | Horizon | 5.5 | 12.1 | 20.0 | 1.37 GB | 57 % | 9 | 0.3 |
| 1 | Vanilla | 18.3 | 27.3 | 20.0 | 3.35 GB | 66 % | 10 | 0.3 |
| 10 | Horizon | 11.5 | 22.1 | 20.0 | 1.41 GB | 124 % | 94 | 2.4 |
| 10 | Vanilla | 16.8 | 30.5 | 20.0 | 3.41 GB | 84 % | 100 | 2.8 |
| 25 | Horizon | 20.7 | 40.4 | 20.0 | 1.45 GB | 170 % | 236 | 5.7 |
| 25 | Vanilla | 16.0 | 26.4 | 20.0 | 3.47 GB | 115 % | 252 | 6.7 |
| 50 | Horizon | 31.4 | 79.0 | 20.0 | 1.52 GB | 173 % | 451 | 10.3 |
| 50 | Vanilla | 20.5 | 29.7 | 20.0 | 3.55 GB | 194 % | 502 | 12.2 |
| 100 | Horizon | 114.5 | 256.2 | 8.7 | 1.33 GB | 181 % | 484 | 30.8 |
| 100 | Vanilla | stopped by its watchdog | | | | | | |

Both servers sent the same chunks (about 10 a second per bot, the churn of flying around) and sent none
twice. Vanilla pulled back a few bots for moving too quickly when a tick ran long (5 to 33 per run);
Horizon none. At 100 bots vanilla's watchdog stopped the server (a tick ran over 60 s, in
`ChunkMap$TrackedEntity.updatePlayer`); Horizon kept running at 8.7 TPS.

### Explore at a rate both servers keep up with (equal work)

The bots fly outwards at 0.8 blocks a second (`--explore-speed 0.8`), slow enough that neither server makes
them wait for terrain, so both generate and send the same new chunks.

| Bots | Server | MSPT | P95 | TPS | RSS | CPU | Chunks/s | New chunks/s |
|---:|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | Horizon | 4.0 | 6.6 | 20.0 | 1.36 GB | 25 % | 2 | 1.3 |
| 1 | Vanilla | 18.1 | 27.1 | 20.0 | 3.32 GB | 73 % | 2 | 1.4 |
| 10 | Horizon | 13.5 | 24.8 | 20.0 | 1.64 GB | 89 % | 13 | 11.8 |
| 10 | Vanilla | 29.2 | 43.0 | 20.0 | 3.52 GB | 143 % | 13 | 11.9 |
| 25 | Horizon | 20.2 | 36.8 | 20.0 | 1.74 GB | 121 % | 32 | 28.9 |
| 25 | Vanilla | 37.0 | 49.4 | 20.0 | 3.73 GB | 169 % | 32 | 28.9 |

### Explore as fast as each server sends chunks (not equal work)

The default 22 blocks a second is faster than either server can generate, so each server sets the pace and
a faster server is given more work: from 10 bots Horizon generated 1.5 to 2 times vanilla's new chunks a
second. Read it as throughput, not as a like-for-like tick time.

| Bots | Server | MSPT | P95 | TPS | RSS | CPU | Chunks/s | New chunks/s |
|---:|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | Horizon | 10.6 | 24.6 | 20.0 | 1.34 GB | 322 % | 39 | 34.6 |
| 1 | Vanilla | 9.9 | 21.3 | 20.0 | 3.48 GB | 245 % | 36 | 33.4 |
| 10 | Horizon | 30.4 | 52.3 | 20.0 | 2.18 GB | 345 % | 47 | 46.7 |
| 10 | Vanilla | 17.0 | 29.6 | 20.0 | 4.04 GB | 242 % | 22 | 22.4 |
| 25 | Horizon | 50.7 | 89.3 | 19.7 | 2.30 GB | 286 % | 50 | 49.7 |
| 25 | Vanilla | 24.2 | 36.5 | 20.0 | 4.40 GB | 320 % | 34 | 33.7 |
| 50 | Horizon | 50.7 | 102.7 | 19.7 | 1.72 GB | 213 % | 97 | 96.5 |
| 50 | Vanilla | stopped by its watchdog | | | | | | |
| 100 | Horizon | 92.9 | 139.6 | 10.8 | 1.70 GB | 322 % | 114 | 113.6 |
| 100 | Vanilla | stopped by its watchdog | | | | | | |

Vanilla's watchdog stopped it at 50 bots (in `ClientboundLightUpdatePacketData`, copying light for a chunk
packet) and at 100 (in `Entity.move` from a move packet).

## Findings

- Horizon's RSS is 1.9 to 2.6 times smaller in every run, and it starts in 11 s against vanilla's 15 s.
  Vanilla's heap is fixed at 4 GB (`-Xms4g`), so part of its RSS is that setting rather than need.
- With 1 bot Horizon's tick is 3.3 times shorter walking and 4.5 times shorter exploring at equal work;
  at 10 bots it is still shorter in both.
- At equal exploring work Horizon's tick is about half of vanilla's at 10 and 25 bots.
- When the bots only fly around spawn (`walk`), vanilla's tick hardly grows with players (16 to 20 ms from
  1 to 50 bots) while Horizon's grows by about 0.5 ms per bot, so vanilla is ahead from about 20 bots: 16.0
  against 20.7 ms at 25 and 20.5 against 31.4 ms at 50. Both keep 20 TPS up to 50.
- With 100 bots vanilla's watchdog stops it in every pass, and Horizon keeps running at 9 to 11 TPS.

### Where Horizon's walk tick goes at 25 bots

Profiled on the ThinkPad with `perf` (the server thread, 20 s, self time):

| Share | What | Whose |
|---:|---|---|
| 18 % | `Streaming::update`: every tick it walks all loaded columns and proto chunks and tests each against every player's area (unloading, dropping protos, finishing and decoration candidates), so the cost grows with players times chunks | streaming / engine tick |
| 20 % | `PalettedContainer::get` and `World::block`, mostly from block entity ticks: on the Mac profile `tick_hoppers` looks up the block under every block entity in the loaded world each tick and compares names as strings, and spawners rebuild their display id | engine tick |
| 8 % | `light_data` when a chunk is sent: the light layers were built twice per chunk; a follow-up commit on #623 builds them once and keeps each section's packed layers | server (this work) |
| 7 % | encoding chunk sections and heightmaps for the chunks each bot newly sees | server |
| 4 % | `Hub::tick` (moves, tracking) | server |
| 2 % | the moved-wrongly collision check | server |

The first two grow with the number of players but aren't specific to the server: they are the
engine's tick and streaming, reported to perf-server-2. Before #623 and #649, encoding every chunk section
once per player and waking each connection's writer thread once per packet cost more than everything above
(at 50 walking bots the tick was 281 ms; see the earlier runs).

## Earlier runs (same machine and method)

- `run-1-before-chunk-cache.json`: before #623, with vanilla's locator bar on. Horizon's walk tick was
  27.5, 40.7, 281 and 467 ms at 10, 25, 50 and 100 bots, and vanilla's 33.2 ms at 50 (the locator bar).
- `run-2-chunk-cache.json`: Horizon with #623 only: 13.2, 23.5, 31.4 and 84.4 ms walking at 10 to 100
  bots.

## Files

- `horizon.json`, `vanilla.json`: the walk and explore passes (vanilla with `locator_bar=false`).
- `explore-equal-rate.json`: the explore pass at 0.8 blocks a second, both servers.
- `run-1-before-chunk-cache.json`, `run-2-chunk-cache.json`: the earlier runs.

To run it again (bots on another machine reached at `--connect`):

    cargo build --release -p horizon-client --bin horizon-client -p horizon-server -p horizon-bots
    python3 tools/server_bench.py --servers horizon,vanilla --bots 1,10,25,50,100 --behaviour walk,explore \
        --gamerule locator_bar=false --nice 19 --bots-via 'ssh macmini' --bots-bin /path/to/horizon-bots \
        --connect <this machine's address> --out results.json
    python3 tools/server_bench.py --servers horizon,vanilla --bots 1,10,25 --behaviour explore \
        --explore-speed 0.8 --gamerule locator_bar=false --nice 19 ... --out explore-equal-rate.json
