# Find the next bottleneck

For Java comparison runs, a private Java Flight Recorder capture helps identify
the code doing work or allocating objects. Use
native Java 25, with `jcmd` and `java` from a JDK. Keep normal benchmark runs separate
from profiling runs because sampling adds overhead.

With the game running and your test world warmed up, find its Java process ID
using Activity Monitor on macOS, Task Manager on Windows, or your process monitor
on Linux. Create the `.local/profiles/` directory inside this checkout first.
Then use a terminal inside the repository:

```sh
# Replace 12345 with the game's Java PID and use your own absolute .local path.
jcmd 12345 JFR.start name=horizon settings=profile duration=120s filename=/absolute/path/to/repo/.local/profiles/mac.jfr
```

Keep the same route, resolution, heap, collector and video settings used for your
timing tests. Wait for the capture to finish, then stream a selected summary:

```sh
java tools/JfrSummary.java .local/profiles/mac.jfr > .local/profiles/mac-summary.json
```

The tool needs only the JDK. It reads events sequentially without expanding every
stack frame's class-loader metadata into a large intermediate JSON dump. Its
selected output does not extract JVM command-line arguments, environment
properties, file events or raw payloads. It includes thread and method labels.
Raw JFR recordings can contain other private
environment data; keep them and their summaries under gitignored `.local/`.

## Interpret the fields

`allocation_threads` contains directly observed per-thread allocation-counter
differences. Each row has its own observation span and count. Thread startup,
exit and intervals outside those counters are not included. A single observation
or unusable interval produces null bytes/rate, rather than a manufactured zero.
Compare observation spans before comparing allocation totals. Allocated bytes
are temporary and retained objects together, not resident RAM or retained heap.

`execution_threads` counts Java execution and native-method stack samples. Leaf
methods locate the sampled frame; inclusive methods count a sample once for each
method appearing anywhere in its stack, including recursion. Samples are hints
for further profiling, not exact CPU time, GPU time or elapsed-time fractions.
Missing/truncated stacks are counted. Method descriptors distinguish overloads.

`allocation_sample_threads` locates allocation stacks by sample count. It does
not sum JFR allocation sample weights. Earlier investigation found those weights
misleading as a substitute for the directly observed thread counters. Different
object types and sampling behavior make sample counts unsuitable for estimating
allocated bytes or proving that one object type owns most of the allocation.

GC pause totals/maximum use top-level `jdk.GCPhasePause`, excluding nested phases
and concurrent collection-cycle duration. `gc_cycles` is a separate count.
`heap_used_peak_at_gc_bytes` considers recorded GC heap summaries and can miss
peaks between collections. It is separate from externally sampled process RSS.

The reader ignores unselected event types and never extracts properties or
arguments. `event_span_seconds` is the span of all observed event timestamps,
which can differ from the requested capture duration; each allocation row reports
its actual counter interval separately.

## Verify the reader

```sh
python3 tools/test_jfr.py
```

This creates a real JDK recording of bounded allocations and GC, then compares
the reader's counter differences, observation counts, GC counts and heap summaries
against `jfr print --json`. It also verifies escaped thread names and that sampled
weights do not appear as numeric allocation totals. The manual `jfr-analysis` CI job runs both collector fixtures on the Linux game runner. The reader was independently checked against the older
real-game profile's JDK JSON export for 85 threads.
