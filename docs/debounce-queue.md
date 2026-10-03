# Debounce queue experiment

The local profile showed substantial allocation on the distant-terrain chunk
update queue thread. Direct per-thread allocation counters confirmed activity
there; sampled stacks repeatedly entered player-position reads and queue
reinsertion. Inspection of the pinned DH 3.3.4 bytecode explains a mechanism:
an update younger than its existing 250 ms debounce deadline is removed and
reinserted, while the outer queue loop sleeps only when both queues are empty.
A nonempty pending queue can therefore retry continuously before it can do work.

The optional `queue-wait` profile keeps balanced graphics, distances, simulation
settings, mods and worker budgets. Its original hook performs the existing
debounce reinsertion, then requests a wait of up to 1 ms, bounded by the remaining
deadline, if no preprocessing work is pending. It only waits on the queue worker,
never a caller such as the render thread. Incoming queued work unparks a debounce
wait; the original empty-queue sleep remains unchanged. OS scheduling determines
when the worker resumes.
The same update object, priority queue, debounce timestamp, overload policy,
executor dispatch and normal simulation remain in use. Executor-unavailable and
rejected-dispatch retries are outside this hook.

`adaptive-queue` combines this hook with the existing adaptive terrain budget
and precise worker pacing. Use `queue-wait` versus `m2-balanced` to isolate the
queue change, and `adaptive-queue` versus `adaptive-pacing` to evaluate the
combination. Each profile explicitly enables only its requested experiments.
Set `enabled=false` in `config/horizon-queue-wait.properties` and restart to
disable the queue hook. It is disabled by default and requires exact DH 3.3.4.

Requested waits are not hard latency bounds: operating-system timers can
overshoot, and arrival/worker scheduling affect responsiveness. Parking leaves
the interruption flag intact, so the existing shutdown loop still handles it.
New arrival wakeups reduce the chance of delaying useful preprocessing, but
real terrain completion, block-change visibility and platform behavior need
evaluation alongside CPU, allocations and frame pacing.

## Isolated measurement

Run `python -c "from tools.build_mod import benchmark_debounce; benchmark_debounce()"`
after building the mod. It uses the actual publisher's `ChunkPosQueue`, with
synthetic entries and a 250 ms pending-debounce interval. The upstream mode
repeatedly pops and reinserts the nearest update; the waiting mode inserts the
same bounded-wait policy between retries. Warm-ups precede three alternating
trials at one and 64 queued entries. Each trial checks update identity, nearest
priority and queue count, then reports wall time, worker CPU and worker allocation.

This benchmark isolates redundant polling; it does not simulate game chunk
lighting, rendering, network traffic or terrain completion. Thread CPU counters
can be coarse, particularly on Windows, so a zero reading is below measurement
resolution, not proof of zero CPU cost. Allocation counts are cumulative bytes
allocated during the interval, not retained heap or process memory. Hardware,
timer behavior and workload can change results. The CI publishes raw output
for Linux, Windows and Apple Silicon macOS. No game-FPS claim follows from this
microbenchmark alone.
