package dev.horizonlab.budget;

import com.seibel.distanthorizons.core.api.internal.chunkUpdating.ChunkPosQueue;
import com.seibel.distanthorizons.core.api.internal.chunkUpdating.ChunkUpdateData;
import com.seibel.distanthorizons.core.pos.DhChunkPos;
import java.util.Locale;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.locks.LockSupport;

/** Isolates wakeup after synthetic incoming work; does not measure terrain rendering or debounce. */
public final class IdleQueueBenchmark {
    private static void run(boolean wakeable, boolean report, int trial) throws Exception {
        var queue = new ChunkPosQueue();
        var update = new ChunkUpdateData(null, null);
        var ready = new CountDownLatch(1);
        var published = new AtomicLong();
        var idleStarted = new AtomicLong();
        var latency = new AtomicLong();
        var failure = new AtomicReference<Throwable>();
        Thread worker = new Thread(() -> {
            try {
                idleStarted.set(System.nanoTime());
                ready.countDown();
                while (queue.isEmpty()) {
                    if (wakeable) WakeableIdleWait.pause(queue, 50_000_000L);
                    else Thread.sleep(50);
                }
                var item = queue.popClosest();
                latency.set(System.nanoTime() - published.get());
                if (item != update || queue.getQueuedCount() != 0)
                    throw new AssertionError("Queued work identity/count changed");
            } catch (Throwable error) { failure.set(error); }
        }, "Horizon idle queue benchmark");
        worker.setDaemon(true);
        worker.start();
        try {
            if (!ready.await(5, TimeUnit.SECONDS)) throw new AssertionError("Worker did not start");
            LockSupport.parkNanos(5_000_000L);
            long start = System.nanoTime();
            published.set(start);
            queue.addItem(new DhChunkPos(0, 0), update);
            if (wakeable) LockSupport.unpark(worker);
            worker.join(5_000);
            if (worker.isAlive()) throw new AssertionError("Incoming work did not reach worker");
            if (failure.get() != null) throw new AssertionError("Worker failed", failure.get());
            if (latency.get() < 0) throw new AssertionError("Invalid timestamp ordering");
            if (report) System.out.printf(Locale.ROOT,
                "mode=%s trial=%d incoming_after_idle_ms=%.3f publication_to_pop_ms=%.3f%n",
                wakeable ? "wakeable" : "upstream", trial, (start - idleStarted.get()) / 1e6, latency.get() / 1e6);
        } finally {
            worker.interrupt();
            worker.join(1_000);
        }
    }

    public static void main(String[] args) throws Exception {
        for (int warmup = 0; warmup < 4; warmup++) { run(false, false, warmup); run(true, false, warmup); }
        for (int trial = 0; trial < 20; trial++) { run(false, true, trial); run(true, true, trial); }
    }
}
