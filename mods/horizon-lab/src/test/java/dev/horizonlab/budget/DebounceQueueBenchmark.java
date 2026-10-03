package dev.horizonlab.budget;

import com.seibel.distanthorizons.core.api.internal.chunkUpdating.ChunkPosQueue;
import com.seibel.distanthorizons.core.api.internal.chunkUpdating.ChunkUpdateData;
import com.seibel.distanthorizons.core.pos.DhChunkPos;
import java.lang.management.ManagementFactory;
import java.util.concurrent.locks.LockSupport;

/** Measures the actual upstream queue during a synthetic pending-debounce interval, not game FPS. */
public final class DebounceQueueBenchmark {
    private static void run(boolean waiting, int size, boolean report) {
        ChunkPosQueue queue = new ChunkPosQueue();
        DhChunkPos closest = new DhChunkPos(0, 0);
        ChunkUpdateData closestData = null;
        for (int i = 0; i < size; i++) {
            ChunkUpdateData update = new ChunkUpdateData(null, null);
            if (i == 0) closestData = update;
            queue.addItem(new DhChunkPos(i, 0), update);
        }
        var threads = (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();
        long id = Thread.currentThread().threadId();
        long allocated = threads.getThreadAllocatedBytes(id), cpu = threads.getCurrentThreadCpuTime();
        long start = System.nanoTime(), debounce = 250_000_000L, loops = 0;
        while (System.nanoTime() - start < debounce) {
            ChunkUpdateData update = queue.popClosest();
            if (update != closestData) throw new AssertionError("Queue priority or data changed");
            queue.addItem(closest, update);
            loops++;
            if (waiting) {
                long delay = DebounceWait.delay(System.nanoTime(), start, debounce, false);
                if (delay > 0) LockSupport.parkNanos(delay);
            }
        }
        long elapsed = System.nanoTime() - start;
        cpu = threads.getCurrentThreadCpuTime() - cpu;
        allocated = threads.getThreadAllocatedBytes(id) - allocated;
        if (queue.getQueuedCount() != size || queue.popClosest() != closestData)
            throw new AssertionError("Pending queue data changed");
        if (report) System.out.printf(java.util.Locale.ROOT,
            "mode=%s size=%d polls=%d worker_cpu_ms=%.3f wall_ms=%.3f worker_allocated_bytes=%d%n",
            waiting ? "wait" : "upstream", size, loops, cpu / 1e6, elapsed / 1e6, allocated);
    }

    public static void main(String[] args) {
        for (int size : new int[]{1, 64}) {
            for (int i = 0; i < 2; i++) { run(false, size, false); run(true, size, false); }
            for (int i = 0; i < 3; i++) { run(false, size, true); run(true, size, true); }
        }
    }
}
