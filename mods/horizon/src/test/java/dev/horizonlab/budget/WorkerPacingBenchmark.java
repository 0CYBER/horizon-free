package dev.horizonlab.budget;

import com.seibel.distanthorizons.core.util.threading.RateLimitedThreadPoolExecutor;
import java.lang.management.ManagementFactory;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicReference;

public class WorkerPacingBenchmark {
    static class PrecisePool extends RateLimitedThreadPoolExecutor {
        final ThreadLocal<dev.horizonlab.budget.WorkerPacer> pacing =
            ThreadLocal.withInitial(dev.horizonlab.budget.WorkerPacer::new);
        PrecisePool(java.util.concurrent.ThreadFactory factory) { super(1, factory, new LinkedBlockingQueue<>()); }
        @Override protected void beforeExecute(Thread thread, Runnable job) { pacing.get().begin(System.nanoTime()); }
        @Override protected void afterExecute(Runnable job, Throwable error) {
            var state = pacing.get();
            long wait = state.finish(System.nanoTime(), runTimeRatioConfig.get());
            if (wait > 0) {
                try { TimeUnit.NANOSECONDS.sleep(wait); state.pauseCompleted(System.nanoTime()); }
                catch (InterruptedException interrupted) { state.interrupted(System.nanoTime()); }
            }
        }
    }
    public static void main(String[] args) throws Exception {
        Class.forName("com.seibel.distanthorizons.core.config.eventHandlers.presets.ThreadPresetConfigEventHandler");
        AtomicReference<Thread> worker = new AtomicReference<>();
        java.util.concurrent.ThreadFactory factory = runnable -> {
            Thread thread = new Thread(runnable, "Horizon probe worker"); worker.set(thread); return thread;
        };
        boolean precise = args.length > 3 && args[3].equals("precise");
        var pool = precise ? new PrecisePool(factory) : new RateLimitedThreadPoolExecutor(1, factory, new LinkedBlockingQueue<>());
        double ratio = Double.parseDouble(args[0]);
        pool.runTimeRatioConfig.setApiValue(ratio, "Horizon probe");
        pool.prestartAllCoreThreads();
        CountDownLatch warmed = new CountDownLatch(500);
        for (int i = 0; i < 500; i++) pool.execute(warmed::countDown);
        warmed.await();
        var bean = (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();
        long id = worker.get().threadId();
        long initialCpu = bean.getThreadCpuTime(id);
        long initialAllocation = bean.getThreadAllocatedBytes(id);
        int tasks = args.length > 2 ? Integer.parseInt(args[2]) : 10_000;
        long workNs = args.length > 1 ? Long.parseLong(args[1]) : 10_000;
        CountDownLatch done = new CountDownLatch(tasks);
        Runnable job = () -> {
            long end = System.nanoTime() + workNs;
            while (System.nanoTime() < end) Thread.onSpinWait();
            done.countDown();
        };
        long started = System.nanoTime();
        for (int i = 0; i < tasks; i++) pool.execute(job);
        done.await();
        long elapsed = System.nanoTime() - started;
        long endCpu = bean.getThreadCpuTime(id);
        long endAllocation = bean.getThreadAllocatedBytes(id);
        long cpu = initialCpu < 0 || endCpu < initialCpu ? -1 : endCpu - initialCpu;
        long allocated = initialAllocation < 0 || endAllocation < initialAllocation ? -1 : endAllocation - initialAllocation;
        pool.shutdown(); pool.awaitTermination(10, TimeUnit.SECONDS);
        String cpuResult = cpu < 0 ? "unavailable" : String.format(java.util.Locale.ROOT, "%.3f", cpu / (double) elapsed);
        System.out.printf(java.util.Locale.ROOT,
            "%s requested ratio %.3f; %d tasks of %d ns; worker CPU/wall %s; %.3f ms; worker allocation %s bytes%n",
            precise ? "precise" : "upstream", ratio, tasks, workNs, cpuResult, elapsed / 1e6,
            allocated < 0 ? "unavailable" : Long.toString(allocated));
    }
}
