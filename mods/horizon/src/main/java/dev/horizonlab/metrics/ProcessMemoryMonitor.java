package dev.horizonlab.metrics;

import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.function.LongSupplier;

/** One background reader for this process, active only while recordings exist. */
public final class ProcessMemoryMonitor {
    private static final Set<Recording> ACTIVE = ConcurrentHashMap.newKeySet();
    private static final ScheduledExecutorService EXECUTOR = Executors.newSingleThreadScheduledExecutor(task -> {
        Thread thread = new Thread(task, "Horizon process memory");
        thread.setDaemon(true);
        thread.setPriority(Thread.MIN_PRIORITY);
        return thread;
    });
    private static volatile LongSupplier reader;
    private static ScheduledFuture<?> polling;

    private ProcessMemoryMonitor() {}

    public static synchronized void configure(LongSupplier residentMemoryReader) {
        if (!ACTIVE.isEmpty()) throw new IllegalStateException("Configure before starting recordings");
        reader = residentMemoryReader;
    }

    static synchronized void track(Recording recording) {
        if (reader == null) return;
        ACTIVE.add(recording);
        if (polling == null) polling = EXECUTOR.scheduleWithFixedDelay(ProcessMemoryMonitor::poll, 0, 1, TimeUnit.SECONDS);
    }

    static synchronized void untrack(Recording recording) {
        ACTIVE.remove(recording);
        if (ACTIVE.isEmpty() && polling != null) {
            polling.cancel(false);
            polling = null;
        }
    }

    private static void poll() {
        Recording[] recordings = ACTIVE.toArray(Recording[]::new);
        if (recordings.length == 0) return;
        long bytes;
        try { bytes = reader.getAsLong(); }
        catch (RuntimeException | LinkageError unavailable) { return; }
        if (bytes <= 0) return;
        for (Recording recording : recordings) recording.residentMemory(bytes);
    }
}
