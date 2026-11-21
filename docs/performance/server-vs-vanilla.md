# Horizon's server benchmarked against alternatives

Horizon’s dedicated server was measured against:
- Vanilla 26.3
- Paper
- Fabric + Lithium

Same bots, world seed and server settings. The run used a four-thread Intel Core i7-7500U laptop with 32 GB of memory;
the servers ran one at a time with Java 25 where applicable.

All four servers were compared at 10, 25 and 50 bots.

## Results

MSPT is the average tick time from each server's own `/tick query`; 50 ms is the 20 TPS budget. 

#### **10 Bots**
| Server | MSPT | P99 | RSS (RAM) | CPU | Start |
| :--- | ---: | ---: | ---: | ---: | ---: |
| **Horizon** | 22.93 ms | **53.33 ms** | **0.92 GB** | **67%** | **8.2 s** |
| **Fabric + Lithium** | **20.18 ms** | 56.08 ms | 3.73 GB | 94% | 16.6 s |
| **Paper** | 20.64 ms | 56.29 ms | 4.07 GB | 74% | 20.5 s |
| **Vanilla** | 22.94 ms | 56.48 ms | 3.71 GB | 101% | 14.7 s |

#### **25 Bots**
| Server | MSPT | P99 | RSS (RAM) | CPU | Start |
| :--- | ---: | ---: | ---: | ---: | ---: |
| **Paper** | **28.30 ms** | 90.40 ms | 4.16 GB | 124% | 19.0 s |
| **Horizon** | 30.92 ms | **72.00 ms** | **1.01 GB** | **99%** | **8.5 s** |
| **Fabric + Lithium** | 31.44 ms | 93.35 ms | 3.88 GB | 174% | 17.5 s |
| **Vanilla** | 37.35 ms | 82.66 ms | 3.94 GB | 191% | 15.5 s |

#### **50 Bots**
| Server | MSPT | P99 | RSS (RAM) | CPU | Start |
| :--- | ---: | ---: | ---: | ---: | ---: |
| **Paper** | **46.86 ms** | 109.72 ms | 4.35 GB | 210% | 19.4 s |
| **Horizon** | 47.83 ms | **73.21 ms** | **1.11 GB** | **143%** | **8.3 s** |
| **Fabric + Lithium** | 58.30 ms | 174.37 ms | 4.52 GB | 236% | 17.5 s |
| **Vanilla** | *Crashed (Watchdog)* | — | — | — | — |

At 50 bots, vanilla's crashed as it fell more than 16 seconds behind. 

Paper is included as a separate optimized-server reference, not as a vanilla-compatible implementation.
It's feature-breaking optimizations mean its results are not directly comparable with Horizon.

## Technical Details

The bots joined one per second, flew in the `mixed` behaviour (walking, exploring and idling), and were measured after
a warm-up. All servers used seed `12345`, view and simulation distance 10, offline mode, creative mode and the same
server properties. The Java servers used a fixed 4 GB heap (`-Xms4g -Xmx4g`); Horizon ran as a native release build.
The bots ran on a separate machine from the server process. The run used Horizon v2026.10.19 at commit `07f2913d`,
Paper 26.3 build 205, and Fabric loader 0.19.5 with Lithium `mc26.3-0.26.2-fabric`. The exact driver invocation,
with only machine-specific transport values redacted, was:

```sh
HORIZON_BENCH_BIN=/path/to/horizon-release-bin \
python3 tools/server_bench.py --servers horizon,vanilla,paper,fabric --bots 1,10,25,50 --behaviour mixed \
  --explore-speed 0.8 --warmup 300 --nice 19 \
  --bots-via 'ssh bots-machine' --bots-bin /path/to/horizon-bots \
  --connect server-address --out four-way.json
```

The full procedure is in [the server benchmark plan](../server.md#9-benchmark-plan-bots-against-vanilla-and-paper),
and the runner is [`tools/server_bench.py`](../../tools/server_bench.py).
