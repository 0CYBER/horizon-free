package dev.horizonlab.budget;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.locks.LockSupport;

public final class WakeableIdleWaitTest {
    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }

    private static void workerCase(boolean interrupt, boolean beforePark) throws Exception {
        var ready = new CountDownLatch(1);
        var proceed = new AtomicBoolean(!beforePark);
        var failure = new AtomicReference<Throwable>();
        var completed = new AtomicBoolean();
        Thread worker = new Thread(() -> {
            ready.countDown();
            // No blocking operation may consume the early unpark permit before the tested wait.
            while (!proceed.get() && !Thread.currentThread().isInterrupted()) Thread.onSpinWait();
            try {
                WakeableIdleWait.pause(completed, TimeUnit.SECONDS.toNanos(30));
                check(!interrupt, "Interrupt must throw");
                completed.set(true);
            } catch (InterruptedException expected) {
                if (!interrupt) failure.set(expected);
                else if (Thread.currentThread().isInterrupted()) failure.set(new AssertionError("Throw must clear interrupt status"));
                else completed.set(true);
            } catch (Throwable error) { failure.set(error); }
        }, "Horizon idle-wait test");
        worker.setDaemon(true);
        worker.start();
        try {
            check(ready.await(5, TimeUnit.SECONDS), "Worker did not start");
            if (!beforePark) {
                long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
                while (worker.getState() != Thread.State.TIMED_WAITING && worker.isAlive() && System.nanoTime() < deadline)
                    Thread.onSpinWait();
                check(worker.getState() == Thread.State.TIMED_WAITING, "Worker did not park");
            }
            if (interrupt) worker.interrupt(); else LockSupport.unpark(worker);
            proceed.set(true);
            worker.join(5_000);
            check(!worker.isAlive(), "Wakeup was lost or shutdown stalled");
            if (failure.get() != null) throw new AssertionError("Worker failed", failure.get());
            check(completed.get(), "Wait did not complete");
        } finally {
            proceed.set(true);
            worker.interrupt();
            worker.join(1_000);
        }
    }

    public static void main(String[] args) throws Exception {
        Thread.currentThread().interrupt();
        try {
            WakeableIdleWait.pause(null, 1_000_000);
            throw new AssertionError("Pending interrupt must throw");
        } catch (InterruptedException expected) {
            check(!Thread.currentThread().isInterrupted(), "Pending interrupt must be cleared");
        }
        workerCase(false, true); // Producer arrived between empty check and park.
        workerCase(false, false); // Producer arrived while worker was idle.
        workerCase(true, true);
        workerCase(true, false);
        System.out.println("Wakeable idle-wait arrival races and interruption tests passed.");
    }
}
