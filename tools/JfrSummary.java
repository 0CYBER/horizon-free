import java.nio.file.Path;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import jdk.jfr.consumer.RecordedEvent;
import jdk.jfr.consumer.RecordedThread;
import jdk.jfr.consumer.RecordingFile;

/** Streams a private JFR without expanding class loaders or exporting command-line arguments. */
public final class JfrSummary {
    static final class Allocation {
        String name;
        Instant firstTime, lastTime;
        long firstBytes, lastBytes, observations;
        boolean invalid;
        void add(Instant time, long bytes, String label) {
            name = label;
            observations++;
            if (bytes < 0) invalid = true;
            if (firstTime == null || time.isBefore(firstTime)) { firstTime = time; firstBytes = bytes; }
            if (lastTime == null || time.isAfter(lastTime)) { lastTime = time; lastBytes = bytes; }
            if (firstTime.equals(lastTime) && firstBytes != lastBytes) invalid = true;
        }
        Map<String, Object> result(long id) {
            var row = new LinkedHashMap<String, Object>();
            row.put("thread_id", id);
            row.put("thread", name);
            row.put("counter_observations", observations);
            double span = Duration.between(firstTime, lastTime).toNanos() / 1e9;
            row.put("observed_seconds", span);
            boolean valid = !invalid && observations > 1 && span > 0 && lastBytes >= firstBytes;
            row.put("observed_allocated_bytes", valid ? lastBytes - firstBytes : null);
            row.put("observed_bytes_per_second", valid ? (lastBytes - firstBytes) / span : null);
            return row;
        }
    }

    static final class Samples {
        String name;
        long count, missingStacks, truncatedStacks;
        final Map<String, Long> leaf = new HashMap<>(), inclusive = new HashMap<>();
        void add(RecordedEvent event, String label) {
            name = label;
            count++;
            var stack = event.getStackTrace();
            if (stack == null || stack.getFrames().isEmpty()) { missingStacks++; return; }
            if (stack.isTruncated()) truncatedStacks++;
            var methods = new java.util.HashSet<String>();
            for (var frame : stack.getFrames()) {
                var method = frame.getMethod();
                methods.add(method.getType().getName() + "." + method.getName() + method.getDescriptor());
            }
            for (String method : methods) inclusive.merge(method, 1L, Long::sum);
            var method = stack.getFrames().getFirst().getMethod();
            leaf.merge(method.getType().getName() + "." + method.getName() + method.getDescriptor(), 1L, Long::sum);
        }
        Map<String, Object> result(long id) {
            return Map.of("thread_id", id, "thread", name, "samples", count,
                "missing_stacks", missingStacks, "truncated_stacks", truncatedStacks,
                "top_leaf_methods", top(leaf), "top_inclusive_methods", top(inclusive));
        }
    }

    static long threadId(RecordedThread thread) {
        return thread.getJavaThreadId() > 0 ? thread.getJavaThreadId() : -thread.getId();
    }

    static String threadName(RecordedThread thread) {
        if (thread.getJavaName() != null) return thread.getJavaName();
        return thread.getOSName() == null ? "unnamed thread" : thread.getOSName();
    }

    static List<Map<String, Object>> top(Map<String, Long> counts) {
        return counts.entrySet().stream()
            .sorted(Map.Entry.<String, Long>comparingByValue().reversed().thenComparing(Map.Entry.comparingByKey()))
            .limit(15).map(e -> Map.<String, Object>of("method", e.getKey(), "samples", e.getValue())).toList();
    }

    static Map<String, Object> summarize(Path path) throws Exception {
        Map<Long, Allocation> allocations = new HashMap<>();
        Map<Long, Samples> execution = new HashMap<>(), allocationSamples = new HashMap<>();
        long events = 0, gcCycles = 0, pauseCount = 0, heapPeak = 0;
        double pauseMs = 0, longestPauseMs = 0;
        Instant first = null, last = null;
        try (var recording = new RecordingFile(path)) {
            while (recording.hasMoreEvents()) {
                var event = recording.readEvent();
                events++;
                Instant time = event.getStartTime();
                if (first == null || time.isBefore(first)) first = time;
                Instant end = event.getEndTime();
                if (last == null || end.isAfter(last)) last = end;
                switch (event.getEventType().getName()) {
                    case "jdk.ThreadAllocationStatistics" -> {
                        RecordedThread thread = event.getThread("thread");
                        if (thread != null) allocations.computeIfAbsent(threadId(thread), k -> new Allocation())
                            .add(time, event.getLong("allocated"), threadName(thread));
                    }
                    case "jdk.ExecutionSample", "jdk.NativeMethodSample", "jdk.ObjectAllocationSample" -> {
                        boolean allocation = event.getEventType().getName().equals("jdk.ObjectAllocationSample");
                        RecordedThread thread = event.getThread(allocation ? "eventThread" : "sampledThread");
                        if (thread != null) (allocation ? allocationSamples : execution)
                            .computeIfAbsent(threadId(thread), k -> new Samples()).add(event, threadName(thread));
                    }
                    case "jdk.GarbageCollection" -> gcCycles++;
                    case "jdk.GCPhasePause" -> {
                        double ms = event.getDuration().toNanos() / 1e6;
                        pauseMs += ms;
                        longestPauseMs = Math.max(longestPauseMs, ms);
                        pauseCount++;
                    }
                    case "jdk.GCHeapSummary" -> heapPeak = Math.max(heapPeak, event.getLong("heapUsed"));
                    default -> { }
                }
            }
        }
        List<Map<String, Object>> allocationRows = new ArrayList<>();
        allocations.forEach((id, allocation) -> allocationRows.add(allocation.result(id)));
        allocationRows.sort(Comparator.<Map<String, Object>>comparingLong(r ->
            r.get("observed_allocated_bytes") instanceof Long bytes ? bytes : -1).reversed()
            .thenComparingLong(r -> (Long) r.get("thread_id")));
        var result = new LinkedHashMap<String, Object>();
        result.put("schema", 1);
        result.put("events_read", events);
        result.put("event_span_seconds", first == null ? 0 : Duration.between(first, last).toNanos() / 1e9);
        result.put("allocation_threads", allocationRows);
        result.put("execution_threads", sampleRows(execution));
        result.put("allocation_sample_threads", sampleRows(allocationSamples));
        result.put("gc_cycles", gcCycles);
        result.put("gc_pause_events", pauseCount);
        result.put("gc_pause_total_ms", pauseMs);
        result.put("gc_pause_longest_ms", longestPauseMs);
        result.put("heap_used_peak_at_gc_bytes", heapPeak == 0 ? null : heapPeak);
        result.put("notes", List.of(
            "Allocation bytes are differences between directly observed per-thread counters, with separate observation spans. Thread startup/exit gaps are not covered; missing intervals are null.",
            "Execution/native-method samples locate sampled stacks, not exact CPU time, GPU work or elapsed-time fractions. Inclusive methods count each sample once even during recursion.",
            "Allocation stack samples are counts only. Sample weights are deliberately excluded; counts are not allocated bytes or retained RAM.",
            "GC pause fields use top-level GCPhasePause events, excluding concurrent cycle duration and nested pause phases. GC heap peaks can miss between-GC peaks.",
            "Profiling changes workload overhead. Keep raw recordings private; they may contain environment details beyond this selected summary."));
        return result;
    }

    static List<Map<String, Object>> sampleRows(Map<Long, Samples> samples) {
        return samples.entrySet().stream().sorted(Comparator.<Map.Entry<Long, Samples>>comparingLong(e -> e.getValue().count)
            .reversed().thenComparingLong(Map.Entry::getKey)).map(e -> e.getValue().result(e.getKey())).toList();
    }

    static String json(Object value) {
        if (value == null) return "null";
        if (value instanceof String string) {
            var out = new StringBuilder("\"");
            for (int i = 0; i < string.length(); i++) {
                char c = string.charAt(i);
                if (c == '"' || c == '\\') out.append('\\').append(c);
                else if (c < 32) out.append(String.format(java.util.Locale.ROOT, "\\u%04x", (int) c));
                else out.append(c);
            }
            return out.append('"').toString();
        }
        if (value instanceof Number || value instanceof Boolean) return value.toString();
        if (value instanceof Map<?, ?> map) return map.entrySet().stream()
            .map(e -> json(e.getKey().toString()) + ":" + json(e.getValue()))
            .collect(java.util.stream.Collectors.joining(",", "{", "}"));
        if (value instanceof List<?> list) return list.stream().map(JfrSummary::json)
            .collect(java.util.stream.Collectors.joining(",", "[", "]"));
        throw new IllegalArgumentException("Unsupported JSON value");
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("Usage: java tools/JfrSummary.java private-recording.jfr");
        System.out.println(json(summarize(Path.of(args[0]))));
    }
}
