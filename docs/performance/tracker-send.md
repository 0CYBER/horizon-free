# Measuring the entity tracker's send pass

PR #14041 originally selected parallel sends at 16 viewers / 64 active entities.
The isolated measurements below support a conservative 400 / 400 cutoff instead.

The ignored `lan::entity::send_bench::tracker_send_benchmark` compares the serial
and parallel send strategies in the same optimized binary. The production
thresholds are 400 viewers and 400 active entities. Normal tests skip it, and `--ignored` alone returns
without measuring: `HORIZON_SEND_BENCH=1` is also required.

## Prepare without running

```sh
cargo test --locked --profile fast -p horizon-client-lib --lib \
  --features tracker-send-bench --no-run
```

Finish compilation, correctness tests and other workloads before measuring.
Record the commit, Rust version, CPU, allowed CPU set, worker count and system
load with the results. Use an otherwise idle machine and the same CPU affinity
for both paths; `HORIZON_SERIAL_TICK=all` must be unset.

## Run only when the machine is available

```sh
HORIZON_WORKER_THREADS=4 HORIZON_SEND_BENCH=1 \
  cargo test --locked --profile fast -p horizon-client-lib --lib \
  --features tracker-send-bench \
  lan::entity::send_bench::tracker_send_benchmark \
  -- --ignored --exact --nocapture --test-threads=1
```

There are seven paired rounds per case. Each round warms both strategies on
identical input, then alternates their execution order for twenty samples.
Every measured tick checks packet bytes, packet order and tracking events for
equality outside the timed region. `SEND_BENCH` lines contain raw JSON samples
in nanoseconds; keep all rounds, including slow ones.

The opt-in feature selects the executable's `horizon_threads::GameAllocator`
for this test binary. It has no effect on normal library builds, and ordinary
tests keep their existing allocator unless the feature is explicitly enabled.

Cases cover both sides of the original thresholds (15/16/17 viewers and
63/64/65 active entities), plus 32, 120, 400 and 600-viewer crowds. Every entity
moves, so the active count is the entity count. The `send` phase measures only
the send pass after tracking is established. `tick_with_pairing` measures the
whole tracker tick, including periodically pairing a receiver again.

Compare distributions and repeated-round medians, not the fastest sample. Set
thresholds only where parallel sends repeatedly improve the measured workload;
retain the serial path below that point. Repeat on a second relevant CPU size
if the crossover depends strongly on the worker count.

This fixture isolates the send strategy. It is not a complete server benchmark:
it excludes world simulation, sockets, inactive entities and the allocation of
the active-index list from the send-only measurement. Before claiming a server
TPS gain, also compare otherwise identical server builds with and without
#14041, using the same template world, bots and CPU allocation in interleaved
runs. Do not attribute the existing combined #14037/#14038/#14041 numbers to
#14041 alone.

After any threshold change, rerun packet-equivalence tests, `pr_ready.py`,
relevant server parity and `lan_e2e.py` before requesting re-review.

## Isolated results, 2026-11-13

Measured commit `06a4eabbc7fc9b0e11f7dabdb403d85b74c4816c`, optimized
`fast` library test binary with `tracker-send-bench` (the game's allocator),
Rust 1.99.1. Host: AMD Ryzen 5 5600X, six physical cores / twelve logical CPUs.
Runs were pinned to physical CPUs 0/1 and 0/1/2/3 (four-core run first), with
worker counts of two and four. No agent-started builds ran beside the measurements. Load
averages at the starts were 3.0 and 5.8 across the twelve logical CPUs; shared-host
activity was not suspended, so these results establish this fixture's crossover,
not an exact prediction for another machine or a complete server.

Each cell is the median of seven round medians, in microseconds. The full
threshold matrix and paired-ratio ranges are in [the CSV](tracker-send-results.csv).

| Workers | Viewers / active | Send serial | Send parallel | Tracker tick serial | Tracker tick parallel |
| --- | --- | --- | --- | --- | --- |
| 2 | 120 / 120 | 146.1 | 184.4 | 306.3 | 340.8 |
| 2 | 400 / 400 | 1802.3 | 1319.1 | 4504.6 | 3609.0 |
| 2 | 600 / 600 | 5602.9 | 2795.0 | 15339.7 | 11927.1 |
| 4 | 120 / 120 | 147.7 | 246.5 | 391.2 | 481.4 |
| 4 | 400 / 400 | 2063.7 | 1515.9 | 4860.3 | 4057.7 |
| 4 | 600 / 600 | 6324.8 | 3260.9 | 16647.4 | 13003.2 |

All byte-order and tracking-event comparisons passed in both runs. At the old
16 / 64 boundary, the parallel whole-tracker median was 2.3–3.3 times slower;
120 / 120 also regressed. At 400 / 400 and 600 / 600, every paired round improved
the whole tracker on both CPU allocations. One four-core 400-player send-only
round was approximately even, while its whole-tracker round still improved.

The cutoff is therefore raised to **400 viewers and 400 active entities**. This
keeps smaller and asymmetric workloads serial and enables the parallel pass only
in the measured crowd range. Counts between 120 and 400 were not measured; this
is a conservative gate, not a claim to have located the exact crossover. The
same-binary strategy comparison includes the dispatch data structures on both
sides; it does not isolate the PR's serial-path active-index allocation from main.
No new end-to-end TPS gain is claimed.
