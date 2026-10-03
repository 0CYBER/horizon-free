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
        check(!snapshot.label().contains("/"), "safe report filename");
        System.out.println("Metrics tests passed.");
    }

    private static void check(boolean condition, String name) {
        if (!condition) throw new AssertionError(name);
    }
}
