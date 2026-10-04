package dev.horizonlab.metrics;

import java.nio.file.Path;
import java.util.Map;

public final class MetricsTest {
    public static void main(String[] args) {
        var sourceContext = new java.util.LinkedHashMap<String, Object>();
        sourceContext.putAll(Map.of("gpu", "GPU", "graphics_backend", "OpenGL", "graphics_driver", "Driver", "private_path", "secret"));
        var serverContext = ServerContext.capture("integrated", sourceContext);
        sourceContext.put("graphics_backend", "Vulkan");
        check(serverContext.get("graphics_backend").equals("OpenGL"), "graphics snapshot does not track later mutations");
        check(!serverContext.containsKey("private_path"), "server context only copies graphics allowlist");
        check(serverContext.get("client_graphics_capture_status").equals("captured_at_recording_start"), "integrated graphics captured");
        check(!ServerContext.capture("dedicated", sourceContext).containsKey("gpu"), "dedicated server does not claim client graphics");
        check(ServerContext.capture("integrated", Map.of("gpu", "GPU")).get("client_graphics_capture_status").equals("not_supplied"), "partial graphics snapshot is unavailable");
        check(ServerContext.capture("unrecognized", Map.of()).get("server_runtime_kind").equals("unknown"), "legacy server kind unknown");
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
        check(!snapshot.metadata().containsKey("tick_state_capture_status"), "frame reports do not pretend to capture server state");
        Recording ticks = new Recording("ticks", "ticks", Path.of("unused"), 10, Map.of());
        ticks.addTick(10_000_000, 1, 20, false, false, false, true);
        ticks.addTick(5_000_000, 1, 40, true, false, false, false);
        ticks.addTick(5_000_000, 1, 20, true, true, true, true);
        ticks.addTick(10_000_000, 1, 20, false, false, false, true);
        var tickSnapshot = ticks.finish();
        ticks.addTick(10_000_000, 99, 40, true, false, true, false);
        var tickMetadata = tickSnapshot.metadata();
        check(tickMetadata.get("tick_state_first").equals(tickMetadata.get("tick_state_last")), "restored boundary states match");
        check(tickMetadata.get("tick_state_changed").equals(java.util.List.of("configured_tick_rate", "frozen", "stepping", "sprinting", "runs_game_elements")),
            "intermediate changes are retained after restoration");
        check(tickMetadata.get("tick_state_samples").equals(4L), "state is sampled with accepted tick durations");
        check(tickMetadata.get("tick_state_frozen_samples").equals(2L), "frozen includes stepping ticks");
        check(tickMetadata.get("tick_state_suppressed_samples").equals(1L), "frozen and simulation suppression are distinct");
        check(tickMetadata.get("tick_state_nonstandard_rate_samples").equals(1L), "custom rate observed between normal boundaries");
        check((double) tickMetadata.get("observed_ticks_per_second") == 4 / (double) tickMetadata.get("elapsed_seconds"),
            "observed TPS uses wall time rather than inverse execution duration");
        check(ticks.finish().metadata().get("tick_state_samples").equals(4L), "late tick observations are ignored");
        Recording unsampled = new Recording("unsampled", "ticks", Path.of("unused"), 10, Map.of());
        unsampled.add(10_000_000);
        check(unsampled.finish().metadata().get("tick_state_capture_status").equals("unavailable"), "missing state is not assumed normal");
        Recording invalid = new Recording("invalid", "ticks", Path.of("unused"), 10, Map.of());
        invalid.addTick(1_000_000, 1, Float.NaN, false, false, false, true);
        invalid.addTick(1_000_000, 1, Float.POSITIVE_INFINITY, false, false, false, true);
        invalid.addTick(1_000_000, 1, 0, false, false, false, true);
        invalid.addTick(1_000_000, 1, 20, false, false, false, true);
        var invalidSnapshot = invalid.finish();
        check(invalidSnapshot.samples().length == 4, "invalid manager observations do not abort tick duration capture");
        check(invalidSnapshot.metadata().get("tick_state_capture_status").equals("invalid_samples"), "invalid state is not silently assumed normal");
        check(invalidSnapshot.metadata().get("tick_state_invalid_samples").equals(3L), "invalid observations counted");
        System.out.println("Metrics tests passed.");
    }

    private static void check(boolean condition, String name) {
        if (!condition) throw new AssertionError(name);
    }
}
