package dev.horizonlab.metrics;

import java.nio.file.Path;
import java.util.Map;

public final class MetricsTest {
    public static void main(String[] args) {
        SampleStats empty = SampleStats.of(new long[0], 0);
        check(empty.count() == 0 && empty.meanRate() == 0, "empty recording");
        SampleStats fixed = SampleStats.of(new long[]{10_000_000, 10_000_000, 10_000_000}, 3);
        check(fixed.meanRate() == 100 && fixed.lowOnePercentRate() == 100, "constant pacing");
        long[] data = new long[100];
        java.util.Arrays.fill(data, 10_000_000);
        data[99] = 100_000_000;
        SampleStats spike = SampleStats.of(data, 100);
        check(spike.p99Ms() == 10 && spike.maxMs() == 100 && spike.lowOnePercentRate() == 10,
            "nearest-rank percentiles and slowest-one-percent mean");
        check(spike.over50Ms() == 1 && spike.over100Ms() == 0, "stall boundaries");
        Recording recording = new Recording("../../recording label", "frames", Path.of("unused"), 120, Map.of());
        for (int i = 0; i < Recording.MAX_SAMPLES + 2; i++) recording.add(10_000_000);
        recording.exclude();
        recording.players(50);
        Recording.Snapshot snapshot = recording.finish();
        check(snapshot.samples().length == Recording.MAX_SAMPLES, "bounded buffer");
        check(snapshot.metadata().get("dropped_samples").equals(2), "overflow reported");
        check(snapshot.metadata().get("excluded_frames").equals(1), "excluded frames reported");
        check(snapshot.metadata().get("peak_players").equals(50), "workload size reported");
        check((long) snapshot.metadata().get("heap_used_peak_sampled_bytes") >=
            (long) snapshot.metadata().get("heap_used_end_bytes"), "heap peak includes the final sample");
        check((int) snapshot.metadata().get("resource_samples") >= 2, "start/end resource snapshots");
        check(snapshot.metadata().get("garbage_collectors") instanceof java.util.List, "collector configuration captured");
        var diagnostic = java.lang.management.ManagementFactory.getPlatformMXBean(com.sun.management.HotSpotDiagnosticMXBean.class);
        if (diagnostic != null) {
            boolean actual = Boolean.parseBoolean(diagnostic.getVMOption("UseCompactObjectHeaders").getValue());
            check(snapshot.metadata().get("compact_object_headers").equals(actual), "header option reflects the actual VM");
        }
        if (snapshot.metadata().containsKey("process_cpu_seconds")) {
            check((double) snapshot.metadata().get("process_cpu_seconds") >= 0, "CPU time is nonnegative");
        }
        check(!snapshot.label().contains("/"), "safe report filename");
        check(!snapshot.metadata().containsKey("process_rss_peak_sampled_bytes"), "unavailable RSS is absent, not zero");
        Recording resident = new Recording("memory", "frames", Path.of("unused"), 10, Map.of());
        resident.residentMemory(-1);
        resident.residentMemory(100);
        resident.residentMemory(200);
        resident.residentMemory(150);
        resident.add(10_000_000);
        var residentSnapshot = resident.finish();
        resident.residentMemory(999);
        resident.add(10_000_000);
        check(residentSnapshot.metadata().get("process_rss_last_sampled_bytes").equals(150L), "last RSS is not called end memory");
        check(residentSnapshot.metadata().get("process_rss_peak_sampled_bytes").equals(200L), "RSS peak can exceed last sample");
        check(residentSnapshot.metadata().get("process_rss_samples").equals(3), "invalid memory samples excluded");
        check(resident.finish().samples().length == 1, "late samples do not change a finished recording");
        System.out.println("Metrics tests passed.");
    }

    private static void check(boolean condition, String name) {
        if (!condition) throw new AssertionError(name);
    }
}
