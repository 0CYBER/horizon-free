# Compare Java collectors

Use the balanced pack first with the launcher's existing JVM policy. Java 25
HotSpot supports explicit G1 (`-XX:+UseG1GC`) and ZGC (`-XX:+UseZGC`) selections;
the [Java command reference](https://docs.oracle.com/en/java/javase/25/docs/specs/man/java.html)
documents both. A collector choice can change CPU, heap headroom, collection
behavior and frame pacing. Lower pauses alone do not establish lower RAM or
higher FPS. The packs do not automatically select a collector.

For a Mac comparison, duplicate one instance. In Edit > Settings > Java, enable
the Java Arguments override checkbox. Preserve existing non-collector arguments,
but use exactly one collector selector in each instance. Choose G1 in one and
ZGC in the other. Use the same native Java 25 ARM64, maximum/minimum heap, compact
header setting, world copy, resolution, shaders, distances and warmed camera route.
Alternate at least three 120-second runs rather than comparing unrelated captures.
Remove the explicit selector to restore the launcher's policy, then restart.

The viewer shows the actual collector names captured by the game. Check them:
setting a launcher's text field does not establish which flags reached the VM.
Prism's per-instance Java Arguments and Memory settings have separate override
checkboxes. A global collector selection may need removal or replacement when
creating an explicit per-instance experiment.

Record FPS, slowest-1% rate, frame spikes, heap and process RSS together. RSS does
not represent all shared GPU memory on Apple Silicon. `gc_time_ms` sums collector
MXBean counters; concurrent cycles and overlapping phases can contribute, so it
is not a measured pause total. Use JFR root pause events for pause analysis.
The streaming reader is checked against the JDK's event exporter with both
default-GC and ZGC fixtures in CI on Windows, Linux and ARM macOS.

The developer demo launcher accepts `--gc default`, `--gc g1` or `--gc zgc`.
For example:

```sh
python3 tools/runtime.py demo --instance .local/runtime/collector-test --heap 6G --gc zgc --compact-headers
python3 tools/test_jfr.py --gc zgc
```

Prepare demo mods/worlds before comparing.
Options affect only the launched process; they do not change global Java settings.
The collector and compact-header experiments can be tested independently.

A local 3 GiB, compact-header ZGC client completed a 120-second game recording.
That recording also encountered a distant-terrain rejection storm. Its workload
and cache history differed from G1 captures, so the larger memory footprint and
different frame rate cannot be attributed to the collector alone. The dispatcher
experiment addresses the identified retry loop separately. No M2 collector
performance winner is established.
