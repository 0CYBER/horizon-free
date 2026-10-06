# Precise distant-terrain worker pacing

The optional `worker-pacing` and `adaptive-pacing` profiles change the timer used by
Distant Horizons background workers. Both keep balanced's 12 full-detail chunks,
128-chunk distant radius, two workers, normal simulation and graphics configuration.
Balanced's timer remains unchanged. `adaptive-pacing` also enables the frame/heap
pressure controller; `worker-pacing` uses the fixed user-configured duty cycle.

The pinned DH 3.3.4 executor measures task duration, calculates a proportional wait,
then converts nanoseconds to whole milliseconds before sleeping. A task that requests
less than a millisecond of waiting reaches `sleep(0)`. Its effect depends on JVM,
operating system and scheduling contention. The configured duty cycle therefore may
not describe what short tasks actually receive. The timestamp is also boxed into a
`Long` per task. Profiling established the boxing site, but did not establish it as
the game's primary allocation cost.

The original Horizon hook accumulates fractional waits until at least 250 microseconds
are due, then sleeps with nanosecond precision. Measured timer overshoot earns credit
against later waits, capped at 50 milliseconds to bound recovery after a long stall.
Primitive state is allocated once per worker rather than boxing
each timestamp. Changes to the duty-cycle setting discard stale debt; an idle worker
also discards it. Interruption during sleep releases debt with the upstream hook's
interruption behavior. The runnable, queues, result, exception and execution order
are unchanged. The inspected direct superclass hooks are no-ops.

This changes background completion time: a worker that previously exceeded a 5%
budget can now take longer to finish the same jobs while leaving more CPU time for
the foreground. On some systems the old zero-duration sleep instead overthrottles
work. Neither less background CPU nor a correct timer guarantees higher game FPS.
OS timer precision and scheduler overhead can still shift actual CPU duty cycle.

The hook is disabled by default and applies only to the exact inspected DH 3.3.4
version. Enable with `config/horizon-worker-pacing.properties`:

```properties
enabled=true
```

Restart the game after changing this property. `/horizon adaptive off` changes the
adaptive controller, not this timer. Frame reports capture `precise_worker_pacing`.

Reproduce the isolated executor benchmark:

```sh
python3 tools/build_mod.py --benchmark-workers
```

`dist/worker-pacing-benchmark.txt` records three runs per mode and requested budget,
using 50,000 jobs with a nominal ten-microsecond workload after warm-up. It compares
the actual upstream executor with a subclass using the original pacing routine.
Worker CPU time and allocation counters exclude allocations on the producer thread.
This is a timer/allocation microbenchmark, not Minecraft FPS, player TPS or a full
terrain generation benchmark. Hosted Linux, macOS and Windows runs produce their
own artifacts; contention and warm-up can change the results.

The first 2,000-job Windows runs exposed CPU-counter quantization: a single worker
could appear above one core or at zero CPU during a very short interval. Those
figures are not reliable CPU-duty results. The longer workload provides a larger
measurement window; counters and scheduling still have finite precision.

Deterministic tests simulate multiple job sizes, timer overshoot, configuration
changes, idle time and invalid ratios. Native game validation must also check terrain
progress, frame/tick capture and clean shutdown. On the M2 Pro, compare balanced,
worker-pacing and adaptive-pacing with identical cold and warm worlds. Record distant
terrain completion times alongside foreground performance and CPU use.

## Hosted executor measurements

[Raw platform results](../benchmarks/worker-pacing-2026-10-03/) came from
[CI run 37147381649](https://github.com/ludovic111/Horizon/actions/runs/37147381649),
using the 50,000-job workload and the revised overshoot credit. Median worker CPU
seconds per elapsed second across three runs were:

| Platform | Requested budget | Upstream | Precise |
| --- | ---: | ---: | ---: |
| macOS ARM64 | 0.50 | 0.620 | 0.514 |
| macOS ARM64 | 0.05 | 0.466 | 0.063 |
| Linux x64 | 0.50 | 1.000 | 0.524 |
| Linux x64 | 0.05 | 0.976 | 0.076 |
| Windows x64 | 0.50 | 0.996 | 0.556 |
| Windows x64 | 0.05 | 0.867 | 0.078 |

These are isolated worker ratios, not game CPU reductions or FPS gains. The macOS
runner exposes three logical CPUs and is not the user's M2 Pro. At the 0.05 setting,
processing the fixed workload takes roughly 10–11 seconds with the precise timer;
many upstream runs finish in under two seconds because the requested delay is not
being applied consistently. Lower background activity costs completion time.
Actual CPU ratios exceed the nominal task-time ratio because executor/sleep overhead
also consumes CPU. OS preemption and counter precision affect each run.

Median worker allocation per job at the 0.50 setting fell from approximately 28–30
bytes to 0.3–1.3 bytes on these runners. This supports removal of the per-job boxed
timestamp; it does not establish a significant whole-game heap reduction.
