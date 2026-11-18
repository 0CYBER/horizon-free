# Horizon's server against vanilla

Horizon’s dedicated server was measured against vanilla 26.3, Paper and Fabric + Lithium with the same deterministic
bots, world seed and server settings. The run used a four-thread Intel Core i7-7500U laptop with 32 GB of memory;
the servers ran one at a time with Java 25 where applicable.

The bots joined one per second, flew in the `mixed` behaviour (walking, exploring and idling), and were measured after
a warm-up. All servers used seed `12345`, view and simulation distance 10, offline mode, creative mode and the same
server properties. The Java servers used a fixed 4 GB heap (`-Xms4g -Xmx4g`); Horizon ran as a native release build.
The bots ran on a separate machine from the server process. The run used Horizon v2026.10.19 at commit `07f2913d`,
Paper 26.3 build 205, and Fabric loader 0.19.5 with Lithium `mc26.3-0.26.2-fabric`. The exact driver invocation,
with only machine-specific transport values redacted, was:

Paper is included as a separate optimized-server reference, not as a vanilla-compatible implementation: its
feature-breaking optimizations mean its results are not directly comparable with Horizon, vanilla or Fabric + Lithium.

```sh
HORIZON_BENCH_BIN=/path/to/horizon-release-bin \
python3 tools/server_bench.py --servers horizon,vanilla,paper,fabric --bots 1,10,25,50 --behaviour mixed \
  --explore-speed 0.8 --warmup 300 --nice 19 \
  --bots-via 'ssh bots-machine' --bots-bin /path/to/horizon-bots \
  --connect server-address --out four-way.json
```

All four servers were compared at 1, 10, 25 and 50 bots.

## Results

MSPT is the average tick time from each server's own `/tick query`; 50 ms is the 20 TPS budget. `Derived TPS*` is
`min(20, 1000 / average MSPT)`, not a count of completed ticks over the wall-clock window. RSS is the mean resident
memory of the server process tree. “New chunks/s” counts chunks the bots had not received before.

| Bots | Server | MSPT | P95 | P99 | Derived TPS* | RSS | CPU | Start | Chunks/s | New chunks/s |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | Horizon | 13.70 | 19.06 | 27.38 | 20.00 | 0.72 GB | 46% | 8.3 s | 9.94 | 0.01 |
| 1 | vanilla | 17.31 | 24.68 | 36.43 | 20.00 | 3.44 GB | 55% | 14.9 s | 9.99 | 0.00 |
| 1 | Paper | 10.31 | 13.90 | 22.50 | 20.00 | 4.24 GB | 32% | 22.3 s | 9.81 | 0.02 |
| 1 | Fabric + Lithium | 12.46 | 17.69 | 32.74 | 20.00 | 3.43 GB | 46% | 17.6 s | 9.98 | 0.00 |
| 10 | Horizon | 22.93 | 39.83 | 53.33 | 20.00 | 0.92 GB | 67% | 8.2 s | 61.21 | 1.78 |
| 10 | vanilla | 22.94 | 36.92 | 56.48 | 20.00 | 3.71 GB | 101% | 14.7 s | 61.63 | 1.84 |
| 10 | Paper | 20.64 | 35.94 | 56.29 | 20.00 | 4.07 GB | 74% | 20.5 s | 60.76 | 1.85 |
| 10 | Fabric + Lithium | 20.18 | 37.04 | 56.08 | 20.00 | 3.73 GB | 94% | 16.6 s | 61.65 | 1.84 |
| 25 | Horizon | 30.92 | 48.25 | 72.00 | 20.00 | 1.01 GB | 99% | 8.5 s | 155.81 | 5.78 |
| 25 | vanilla | 37.35 | 48.20 | 82.66 | 20.00 | 3.94 GB | 191% | 15.5 s | 158.52 | 5.71 |
| 25 | Paper | 28.30 | 61.68 | 90.40 | 20.00 | 4.16 GB | 124% | 19.0 s | 156.35 | 5.72 |
| 25 | Fabric + Lithium | 31.44 | 52.45 | 93.35 | 20.00 | 3.88 GB | 174% | 17.5 s | 158.89 | 5.75 |
| 50 | Horizon | 47.83 | 62.42 | 73.21 | 20.00 | 1.11 GB | 143% | 8.3 s | 302.68 | 11.02 |
| 50 | vanilla | watchdog stopped the server | — | — | — | — | — | — | — | — |
| 50 | Paper | 46.86 | 61.16 | 109.72 | 20.00 | 4.35 GB | 210% | 19.4 s | 310.51 | 11.16 |
| 50 | Fabric + Lithium | 58.30 | 83.06 | 174.37 | 17.15 | 4.52 GB | 236% | 17.5 s | 311.52 | 11.28 |

At 50 bots, vanilla's watchdog stopped the server during warm-up after it fell more than 16 seconds behind. Horizon
averaged 47.83 ms per tick in this run. The comparison is a measurement on one machine, not a claim that every
workload or computer will produce the same result.

The full procedure is in [the server benchmark plan](../server.md#9-benchmark-plan-bots-against-vanilla-and-paper),
and the runner is [`tools/server_bench.py`](../../tools/server_bench.py).
