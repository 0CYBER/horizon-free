# Measuring resident process memory

Horizon 0.6.0 records process RSS alongside JVM heap and nonheap pools. RSS
counts resident pages belonging to the Minecraft process, including native
allocations and mapped pages. It helps detect cases where reducing heap use
merely moves memory elsewhere. It is not unique physical-memory ownership or
a complete accounting of GPU allocations, shared pages, compressed memory,
swap, or Apple Silicon unified memory. Do not add heap to RSS: heap pages can
already be included in it.

Minecraft 26.3 already ships OSHI 6.9.0 and JNA on both client and server. The
original mod uses those dependencies without embedding them. Native reader
initialization and refresh happen on a low-priority daemon thread, never on
the render or server-tick thread. One reader serves overlapping frame and
tick recordings. Reads run about once per second only while recordings are
active; the scheduler stops polling when the last recording finishes. Ordinary
gameplay without a benchmark performs no resident-memory reads.

Reports include these optional schema-1 fields:

| Field | Meaning |
| --- | --- |
| `process_rss_peak_sampled_bytes` | Largest successful resident-memory sample during this recording |
| `process_rss_last_sampled_bytes` | Last successful sample, which can precede recording completion |
| `process_rss_samples` | Number of successful samples delivered to this recording |
| `process_rss_sample_interval_ms` | Requested 1,000 ms interval; reads can take longer |
| `process_rss_last_sample_age_ms` | Time from the last successful sample to report completion |

Unsupported or failed reads leave these fields absent if no sample succeeded;
zero is not used as a substitute. Frames and ticks continue recording. A read
that completes after a recording finishes is ignored. Sampled peaks can miss
brief spikes, and the initial read can be delayed by native-library startup.

The offline viewer compares RSS separately from heap/nonheap and warns about
stale resident samples. Keep recording duration, workload and warm-up matched
when comparing peaks. Process CPU counters cover the whole process over the
recording wall time, including native work and the integrated server; they do
not identify which thread or mod caused CPU use.

`gc_time_ms` sums collector MXBean collection-time counters. With concurrent
collectors these can include cycle duration and overlapping phases; it is not a
direct stop-the-world pause measurement. The viewer calls it collection time.
Use [JFR pause events](profiling.md) when investigating GC-induced stalls.

A three-pair heap experiment compared 3 GiB and 1.5 GiB
maximum heaps with the same core stack. It recorded variable RSS savings, nearly
unchanged average FPS and increased GC accounting; playable defaults stayed
unchanged.

The native-read test exercises the real library, overlapping recordings,
background execution, idle cancellation and unavailable-reader behavior on
the build platform. This validates measurement availability, not an FPS or
RAM optimization result.
