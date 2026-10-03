package dev.horizonlab.metrics;

import java.nio.file.Path;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

/** Real native reads on each CI platform; also checks idle and failure behavior. */
public final class ResidentMemoryTest {
    public static void main(String[] args) throws Exception {
        ResidentMemory reader = new ResidentMemory();
        AtomicInteger calls = new AtomicInteger();
        CountDownLatch threeReads = new CountDownLatch(3);
        Thread caller = Thread.currentThread();
        ProcessMemoryMonitor.configure(() -> {
            if (Thread.currentThread() == caller) throw new AssertionError("Memory read on recording thread");
            long rss = reader.getAsLong();
            if (rss <= 0) throw new AssertionError("Native resident memory unavailable on this runner");
            calls.incrementAndGet();
            threeReads.countDown();
            return rss;
        });
        try {
            Thread.sleep(50);
            if (calls.get() != 0) throw new AssertionError("Reader ran without a recording");
            Recording frames = new Recording("rss", "frames", Path.of("unused"), 10, Map.of());
            Recording ticks = new Recording("rss", "ticks", Path.of("unused"), 10, Map.of());
            // The third read starts after the second dispatch completed, so both recordings have data.
            if (!threeReads.await(10, TimeUnit.SECONDS)) throw new AssertionError("Background reads did not complete");
            var frameReport = frames.finish();
            var tickReport = ticks.finish();
            for (var report : new Recording.Snapshot[]{frameReport, tickReport}) {
                var metadata = report.metadata();
                long last = (long) metadata.get("process_rss_last_sampled_bytes");
                long peak = (long) metadata.get("process_rss_peak_sampled_bytes");
                if (last <= 0 || peak < last) throw new AssertionError("Invalid resident memory report");
                System.out.println(report.kind() + " RSS last=" + last + " peak=" + peak + " samples=" + metadata.get("process_rss_samples"));
            }
            int idleCalls = calls.get();
            Thread.sleep(1100);
            if (calls.get() != idleCalls) throw new AssertionError("Reader kept polling after recordings finished");
            CountDownLatch failedRead = new CountDownLatch(1);
            ProcessMemoryMonitor.configure(() -> {
                failedRead.countDown();
                throw new IllegalStateException("Unsupported native reader");
            });
            Recording unavailable = new Recording("unsupported", "frames", Path.of("unused"), 10, Map.of());
            unavailable.add(1_000_000);
            if (!failedRead.await(10, TimeUnit.SECONDS)) throw new AssertionError("Unavailable reader was not attempted");
            var unavailableReport = unavailable.finish();
            if (unavailableReport.samples().length != 1 || unavailableReport.metadata().containsKey("process_rss_peak_sampled_bytes"))
                throw new AssertionError("Unavailable memory disrupted recording or was reported as zero");
            System.out.println("Background resident-memory checks passed.");
        } finally {
            ProcessMemoryMonitor.configure(null);
        }
    }
}
