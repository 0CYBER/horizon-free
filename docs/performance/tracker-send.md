# Measuring the entity tracker's send pass

PR #14041 needs an isolated comparison before its 16-viewer / 64-active-entity
thresholds can be accepted. No new performance results are claimed here.

The ignored `lan::entity::send_bench::tracker_send_benchmark` compares the serial
and parallel send strategies in the same optimized binary. The production
thresholds stay unchanged. Normal tests skip it, and `--ignored` alone returns
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

Cases cover both sides of the current thresholds (15/16/17 viewers and
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
