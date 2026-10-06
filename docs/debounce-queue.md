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
wait. Starting with 0.9, incoming work can also wake an empty queue's idle wait,
which previously slept for 50 ms. The idle park ends by the next 50 ms cache
maintenance deadline; early wakeups do not trigger maintenance more often than
that cadence. OS scheduling determines when the worker resumes.
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
overshoot, and arrival/worker scheduling affect responsiveness. Debounce parking
leaves the interruption flag intact, so the existing shutdown loop handles it.
The wakeable idle wait throws with a cleared interrupt flag, as `Thread.sleep`
did; the original catch restores the flag and exits on the next loop check.
New arrival wakeups reduce the chance of delaying useful preprocessing, but
real terrain completion, block-change visibility and platform behavior need
evaluation alongside CPU, allocations and frame pacing.

## Idle wakeup experiment in 0.9

After building, run
`python -c "from tools.build_mod import benchmark_idle; benchmark_idle()"`.
An actual publisher queue starts empty; synthetic incoming work is published
about 5 ms into an idle interval. Separate upstream-sleep and wakeable-park workers
check the same update identity and empty queue after removal. Four warm-up pairs
precede 20 alternating measured pairs. Output reports actual arrival time and
publication-start-to-pop latency. Local Linux medians were 45.181 ms upstream and
0.199 ms wakeable.

Hosted medians and nearest-rank p95 over 20 measured trials:

| Platform | Upstream median | Wakeable median | Wakeable p95 | Wakeable maximum |
| --- | ---: | ---: | ---: | ---: |
| macOS ARM64 | 84.273 ms | 0.068 ms | 0.103 ms | 16.096 ms |
| Linux x64 | 45.065 ms | 0.053 ms | 0.073 ms | 0.073 ms |
| Windows x64 | 45.159 ms | 0.074 ms | 0.127 ms | 0.182 ms |

The upstream Mac maximum was 108.892 ms; its requested 50 ms sleep overshot.
The wakeable Mac maximum also shows scheduler variability. The hosted ARM Mac
has three logical CPUs and is not the user's M2 Pro. [Raw trials](../benchmarks/idle-queue-2026-10-03)
are from [CI run 37155695460](https://github.com/ludovic111/Horizon/actions/runs/37155695460).

This isolates idle wakeup, including enqueue/pop overhead. It does not simulate
the existing 250 ms debounce, preprocessing, chunk generation, meshing, network
delivery or a visible block update. Those delays remain separate. OS scheduling
can change latency; this is not an FPS measurement or a hard responsiveness bound.
Concurrency tests cover arrival before park, arrival while parked, pending
interrupts and interruption while parked. The runtime hook still requires exact
DH 3.3.4 and is enabled only with the existing queue experiment.

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

## Hosted results, 3 October 2026

Raw output is checked in under
[`benchmarks/debounce-queue-2026-10-03`](../benchmarks/debounce-queue-2026-10-03/).
The tables show medians of three trials. Allocations and CPU refer only to this
synthetic queue polling interval, not the entire game. The macOS runner is hosted
Apple Silicon with three logical CPUs; it is not the user's M2 Pro.

| Platform / queued entries | Upstream allocation | Waiting allocation | Upstream worker CPU | Waiting worker CPU |
| --- | ---: | ---: | ---: | ---: |
| macOS ARM64 / 1 | 80,066,144 B | 768 B | 247.157 ms | 0.873 ms |
| macOS ARM64 / 64 | 34,344,320 B | 2,112 B | 248.550 ms | 1.154 ms |
| Linux x64 / 1 | 99,456,064 B | 7,520 B | 248.446 ms | 2.960 ms |
| Linux x64 / 64 | 31,971,072 B | 7,488 B | 249.987 ms | 2.890 ms |
| Windows x64 / 1 | 95,728,256 B | 5,536 B | 250.000 ms | Below counter resolution |
| Windows x64 / 64 | 32,051,200 B | 5,376 B | 250.000 ms | Below counter resolution |

Each upstream interval lasted about 250 ms. Waiting medians were 250.068 ms on
Linux, 250.593–250.856 ms on Windows, and 251.431–252.426 ms on macOS. One macOS
trial lasted 258.012 ms, demonstrating timer overshoot. These are interval-ending
times; they are not measurements of a real block change reaching the screen.

The real 0.7.0 adaptive-queue demo world renders and records with the hook enabled.
Thread inspection reached the injected debounce parking path. A 120-second run
wrote 643 frame intervals and 2,408 server ticks with matching CSV counts/means,
and 119–120 RSS samples. Distant-terrain data continued updating; game worlds and
DH databases closed cleanly. A separate 60-second profile confirmed allocation
counter activity on the queue thread. Its workload/cache state differs from the
earlier profile, so those profiles are not a numerical before/after comparison.

## Stationary live allocation observation, 4 October 2026

A [selected real-game pair](../benchmarks/debounce-queue-2026-10-04/live-observations.json)
uses separate copies of the same private world, matching graphics/JVM settings
and first saved cameras, with the released 0.18 production JAR. Both captures
have zero exclusions/drops and both worlds save and exit with code zero. JFR
starts roughly 121 seconds after demo dismissal; the later 120-second frame
capture starts roughly 307 seconds after dismissal.

Overworld queue allocation counters record about 15.3 MB/s upstream versus
11.5 KB/s with waiting across their roughly 64-second spans. This supports a
large targeted allocation difference in this observation. Work/geometry volumes
are not quantitatively controlled, and daylight, entities and cache/JIT state
can differ. The later control capture has 230 frames versus 225 with waiting;
sampled peak RSS is lower in control and whole-process CPU differs little.
No useful game FPS, retained-memory or whole-process CPU gain is established.
This is Linux software rendering, not an Apple Silicon test. The hook remains
optional, and actual terrain update latency still needs hardware measurement.

Both JSON/CSV viewer charts and the 390-pixel layout pass. An earlier failed
capture had zero eligible frames and 209 exclusions after the virtual keyboard
ended and the game paused. The viewer rejects that real report, clears prior
comparison results and asks for focused gameplay; it is excluded from the pair.
