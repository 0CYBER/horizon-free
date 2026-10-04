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
    private final java.lang.management.MemoryMXBean memory = ManagementFactory.getMemoryMXBean();
    private final com.sun.management.OperatingSystemMXBean operatingSystem =
        ManagementFactory.getOperatingSystemMXBean() instanceof com.sun.management.OperatingSystemMXBean bean ? bean : null;
    private final long initialCpuNs;
    private long nextResourceSampleNs;
    private long peakHeap;
    private long peakNonHeap;
    private int resourceSamples;
    private long residentEnd;
    private long residentPeak;
    private int residentSamples;
    private long residentSampleNs;
    private boolean finished;
    private final TickState tickState;

    public Recording(String label, String kind, Path output, int seconds, Map<String, Object> context) {
        if (seconds < 1 || seconds > 600) throw new IllegalArgumentException("Duration must be 1..600 seconds");
        String safeLabel = label.replaceAll("[^A-Za-z0-9_-]", "_");
        this.label = safeLabel.substring(0, Math.min(safeLabel.length(), 64));
        this.kind = kind;
        this.tickState = kind.equals("ticks") ? new TickState() : null;
        this.output = output;
        this.context = new LinkedHashMap<>(context);
        this.startedNs = System.nanoTime();
        this.deadlineNs = startedNs + seconds * 1_000_000_000L;
        this.initialGcCount = gcCount();
        this.initialGcMs = gcMs();
        this.initialCpuNs = operatingSystem == null ? -1 : operatingSystem.getProcessCpuTime();
        sampleResources(startedNs);
        ProcessMemoryMonitor.track(this);
    }

    public synchronized void add(long ns) {
        if (ns <= 0 || finished) return;
        long now = System.nanoTime();
        if (now >= nextResourceSampleNs) sampleResources(now);
        if (count == samples.length) { dropped++; return; }
        samples[count++] = ns;
    }

    public synchronized void exclude() { excluded++; }
    public synchronized void players(int players) { peakPlayers = Math.max(players, peakPlayers); }

    public synchronized void addTick(long ns, int players, float rate, boolean frozen, boolean stepping, boolean sprinting, boolean runsGameElements) {
        if (ns <= 0 || finished || !kind.equals("ticks")) return;
        tickState.observe(rate, frozen, stepping, sprinting, runsGameElements);
        add(ns);
        players(players);
    }

    public synchronized Snapshot finish() {
        finished = true;
        ProcessMemoryMonitor.untrack(this);
        sampleResources(System.nanoTime());
        Map<String, Object> metadata = new LinkedHashMap<>(context);
        metadata.put("label", label);
        metadata.put("kind", kind);
        metadata.put("schema", 1);
        double elapsed = (System.nanoTime() - startedNs) / 1e9;
        metadata.put("elapsed_seconds", elapsed);
        if (kind.equals("ticks")) {
            metadata.put("observed_ticks_per_second", (count + (long) dropped) / elapsed);
            metadata.putAll(tickState.metadata());
        }
        metadata.put("dropped_samples", dropped);
        metadata.put("excluded_frames", excluded);
        metadata.put("peak_players", peakPlayers);
        metadata.put("java_version", System.getProperty("java.version"));
        metadata.put("os", System.getProperty("os.name"));
        metadata.put("os_version", System.getProperty("os.version"));
        metadata.put("architecture", System.getProperty("os.arch"));
        metadata.put("logical_processors", Runtime.getRuntime().availableProcessors());
        metadata.put("garbage_collectors", ManagementFactory.getGarbageCollectorMXBeans().stream()
            .map(java.lang.management.MemoryManagerMXBean::getName).sorted().toList());
        try {
            var diagnostic = ManagementFactory.getPlatformMXBean(com.sun.management.HotSpotDiagnosticMXBean.class);
            if (diagnostic != null) metadata.put("compact_object_headers",
                Boolean.parseBoolean(diagnostic.getVMOption("UseCompactObjectHeaders").getValue()));
        } catch (RuntimeException | LinkageError unavailable) {
            // Other VM implementations may not expose this option; absence is distinct from false.
        }
        var endHeap = memory.getHeapMemoryUsage();
        peakHeap = Math.max(peakHeap, endHeap.getUsed());
        metadata.put("heap_used_end_bytes", endHeap.getUsed());
        metadata.put("heap_committed_end_bytes", endHeap.getCommitted());
        metadata.put("heap_max_bytes", Runtime.getRuntime().maxMemory());
        metadata.put("heap_used_peak_sampled_bytes", peakHeap);
        metadata.put("nonheap_used_peak_sampled_bytes", peakNonHeap);
        metadata.put("resource_samples", resourceSamples);
        metadata.put("resource_sample_interval_ms", 1000);
        if (residentSamples > 0) {
            metadata.put("process_rss_last_sampled_bytes", residentEnd);
            metadata.put("process_rss_peak_sampled_bytes", residentPeak);
            metadata.put("process_rss_samples", residentSamples);
            metadata.put("process_rss_sample_interval_ms", 1000);
            metadata.put("process_rss_last_sample_age_ms", (System.nanoTime() - residentSampleNs) / 1e6);
        }
        long endCpuNs = operatingSystem == null ? -1 : operatingSystem.getProcessCpuTime();
        if (initialCpuNs >= 0 && endCpuNs >= initialCpuNs) {
            double cpuSeconds = (endCpuNs - initialCpuNs) / 1e9;
            metadata.put("process_cpu_seconds", cpuSeconds);
            metadata.put("process_cpu_mean_cores", cpuSeconds / elapsed);
        }
        metadata.put("gc_collections", gcCount() - initialGcCount);
        metadata.put("gc_time_ms", gcMs() - initialGcMs);
        return new Snapshot(label, kind, output, Arrays.copyOf(samples, count), metadata);
    }

    synchronized void residentMemory(long bytes) {
        if (finished || bytes <= 0) return;
        residentEnd = bytes;
        residentPeak = Math.max(residentPeak, bytes);
        residentSamples++;
        residentSampleNs = System.nanoTime();
    }

    private void sampleResources(long now) {
        peakHeap = Math.max(peakHeap, memory.getHeapMemoryUsage().getUsed());
        peakNonHeap = Math.max(peakNonHeap, memory.getNonHeapMemoryUsage().getUsed());
        resourceSamples++;
        nextResourceSampleNs = now + 1_000_000_000L;
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
