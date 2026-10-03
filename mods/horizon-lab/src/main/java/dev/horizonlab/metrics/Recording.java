package dev.horizonlab.metrics;

import java.lang.management.ManagementFactory;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.Map;

/** One preallocated bounded buffer per recording, no per-sample allocations or disk I/O. */
public final class Recording {
    public static final int MAX_SAMPLES = 300_000;
    public final String label;
    public final String kind;
    public final Path output;
    public final long startedNs;
    public final long deadlineNs;
    public final Map<String, Object> context;
    private final long[] samples = new long[MAX_SAMPLES];
    private int count;
    private int dropped;
    private int excluded;
    private int peakPlayers;
    private final long initialGcCount;
    private final long initialGcMs;

    public Recording(String label, String kind, Path output, int seconds, Map<String, Object> context) {
        if (seconds < 1 || seconds > 600) throw new IllegalArgumentException("Duration must be 1..600 seconds");
        String safeLabel = label.replaceAll("[^A-Za-z0-9_-]", "_");
        this.label = safeLabel.substring(0, Math.min(safeLabel.length(), 64));
        this.kind = kind;
        this.output = output;
        this.context = new LinkedHashMap<>(context);
        this.startedNs = System.nanoTime();
        this.deadlineNs = startedNs + seconds * 1_000_000_000L;
        this.initialGcCount = gcCount();
        this.initialGcMs = gcMs();
    }

    public synchronized void add(long ns) {
        if (ns <= 0) return;
        if (count == samples.length) { dropped++; return; }
        samples[count++] = ns;
    }

    public synchronized void exclude() { excluded++; }
    public synchronized void players(int players) { peakPlayers = Math.max(players, peakPlayers); }

    public synchronized Snapshot finish() {
        Map<String, Object> metadata = new LinkedHashMap<>(context);
        metadata.put("label", label);
        metadata.put("kind", kind);
        metadata.put("schema", 1);
        double elapsed = (System.nanoTime() - startedNs) / 1e9;
        metadata.put("elapsed_seconds", elapsed);
        if (kind.equals("ticks")) metadata.put("observed_ticks_per_second", (count + (long) dropped) / elapsed);
        metadata.put("dropped_samples", dropped);
        metadata.put("excluded_frames", excluded);
        metadata.put("peak_players", peakPlayers);
        metadata.put("java_version", System.getProperty("java.version"));
        metadata.put("os", System.getProperty("os.name"));
        metadata.put("os_version", System.getProperty("os.version"));
        metadata.put("architecture", System.getProperty("os.arch"));
        metadata.put("logical_processors", Runtime.getRuntime().availableProcessors());
        metadata.put("heap_used_end_bytes", ManagementFactory.getMemoryMXBean().getHeapMemoryUsage().getUsed());
        metadata.put("heap_committed_end_bytes", ManagementFactory.getMemoryMXBean().getHeapMemoryUsage().getCommitted());
        metadata.put("heap_max_bytes", Runtime.getRuntime().maxMemory());
        metadata.put("gc_collections", gcCount() - initialGcCount);
        metadata.put("gc_time_ms", gcMs() - initialGcMs);
        return new Snapshot(label, kind, output, Arrays.copyOf(samples, count), metadata);
    }

    private static long gcCount() {
        return ManagementFactory.getGarbageCollectorMXBeans().stream()
            .mapToLong(gc -> Math.max(0, gc.getCollectionCount())).sum();
    }

    private static long gcMs() {
        return ManagementFactory.getGarbageCollectorMXBeans().stream()
            .mapToLong(gc -> Math.max(0, gc.getCollectionTime())).sum();
    }

    public record Snapshot(String label, String kind, Path output, long[] samples, Map<String, Object> metadata) {}
}
