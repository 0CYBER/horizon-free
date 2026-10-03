package dev.horizonlab.dispatch;

import com.seibel.distanthorizons.core.util.threading.PriorityTaskPicker;
import com.seibel.distanthorizons.core.util.threading.RateLimitedThreadPoolExecutor;
import dev.horizonlab.mixin.HorizonMixinPlugin;
import java.lang.reflect.Field;
import java.lang.management.ManagementFactory;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;

/** Private fixture mod, excluded from the production JAR. Run inside Fabric to test applied hooks. */
public final class DispatchIntegration implements ClientModInitializer {
    @Override public void onInitializeClient() {
        ClientLifecycleEvents.CLIENT_STARTED.register(client -> {
            try { exercise(HorizonMixinPlugin.dispatchRetryEnabled()); }
            catch (Exception failure) { throw new AssertionError("Dispatch integration failed", failure); }
            if (Boolean.getBoolean("horizon.fixture.exit")) System.exit(0);
        });
    }

    private static void exercise(boolean enabled) throws Exception {
        PriorityTaskPicker picker = new PriorityTaskPicker();
        var blocked = picker.createExecutor("Horizon fixture blocked", 1);
        var independent = picker.createExecutor("Horizon fixture independent", 2);
        Field poolField = blocked.getClass().getDeclaredField("threadPoolExecutor");
        poolField.setAccessible(true);
        ((RateLimitedThreadPoolExecutor) poolField.get(blocked)).shutdownNow();
        var physical = new RateLimitedThreadPoolExecutor(1, runnable -> {
            Thread thread = new Thread(runnable, "Horizon fixture physical");
            thread.setDaemon(true);
            return thread;
        }, new ArrayBlockingQueue<Runnable>(1));
        AtomicInteger rejections = new AtomicInteger();
        var abort = new ThreadPoolExecutor.AbortPolicy();
        physical.setRejectedExecutionHandler((task, pool) -> {
            rejections.incrementAndGet();
            abort.rejectedExecution(task, pool);
        });
        poolField.set(blocked, physical);
        CountDownLatch entered = new CountDownLatch(1), release = new CountDownLatch(1);
        CountDownLatch submitted = new CountDownLatch(1), completed = new CountDownLatch(2);
        AtomicInteger firstRuns = new AtomicInteger(), secondRuns = new AtomicInteger();
        AtomicLong callerBytes = new AtomicLong(), callerCpu = new AtomicLong(), callerWall = new AtomicLong();
        AtomicReference<Throwable> callerFailure = new AtomicReference<>();
        var threads = (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();
        threads.setThreadAllocatedMemoryEnabled(true);
        threads.setThreadCpuTimeEnabled(true);
        try {
            // Physical work is deliberately outside the picker: reproduce a busy executor with free logical slots.
            physical.execute(() -> {
                entered.countDown();
                try { release.await(10, TimeUnit.SECONDS); }
                catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
            });
            if (!entered.await(5, TimeUnit.SECONDS)) throw new AssertionError("Physical worker did not start");
            physical.execute(() -> {}); // Fill its only physical queue slot.
            Thread caller = new Thread(() -> {
                long id = Thread.currentThread().threadId();
                long bytes = threads.getThreadAllocatedBytes(id), cpu = threads.getCurrentThreadCpuTime(), wall = System.nanoTime();
                try { blocked.execute(() -> { firstRuns.incrementAndGet(); completed.countDown(); }); }
                catch (Throwable failure) { callerFailure.set(failure); }
                finally {
                    callerWall.set(System.nanoTime() - wall);
                    callerBytes.set(threads.getThreadAllocatedBytes(id) - bytes);
                    callerCpu.set(threads.getCurrentThreadCpuTime() - cpu);
                    submitted.countDown();
                }
            }, "Horizon fixture caller");
            caller.setDaemon(true);
            caller.start();
            boolean returned = submitted.await(enabled ? 2000 : 200, TimeUnit.MILLISECONDS);
            if (returned != enabled) throw new AssertionError("Unexpected rejection lock behavior: hook=" + enabled);
            independent.execute(() -> { secondRuns.incrementAndGet(); completed.countDown(); });
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
            if (enabled) {
                while (secondRuns.get() == 0 && System.nanoTime() < deadline) Thread.sleep(1);
                if (secondRuns.get() != 1 || firstRuns.get() != 0) throw new AssertionError("Blocked executor prevented independent work");
            } else if (secondRuns.get() != 0) throw new AssertionError("Upstream did not reproduce dispatcher blocking");
            release.countDown();
            if (!completed.await(5, TimeUnit.SECONDS)) throw new AssertionError("Requeued task was lost");
            caller.join(5000);
            if (caller.isAlive() || callerFailure.get() != null) throw new AssertionError("Submission failed", callerFailure.get());
            Field occupiedField = PriorityTaskPicker.class.getDeclaredField("occupiedThreadsRef");
            occupiedField.setAccessible(true);
            AtomicInteger occupied = (AtomicInteger) occupiedField.get(picker);
            deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
            while (occupied.get() != 0 && System.nanoTime() < deadline) Thread.sleep(1);
            if (occupied.get() != 0 || firstRuns.get() != 1 || secondRuns.get() != 1
                    || blocked.getQueueSize() != 0 || independent.getQueueSize() != 0)
                throw new AssertionError("Task count, queue, or occupied slots corrupted");
            System.out.println("HORIZON_DISPATCH_RESULT {\"enabled\":" + enabled
                + ",\"submission_allocated_bytes\":" + callerBytes.get()
                + ",\"submission_cpu_ns\":" + callerCpu.get() + ",\"submission_wall_ns\":" + callerWall.get()
                + ",\"rejections\":" + rejections.get() + ",\"tasks_completed\":2,\"occupied_slots\":0}");
        } finally {
            release.countDown();
            picker.shutdownNow();
            if (!physical.awaitTermination(5, TimeUnit.SECONDS)) throw new AssertionError("Fixture worker did not shut down");
        }
    }
}
