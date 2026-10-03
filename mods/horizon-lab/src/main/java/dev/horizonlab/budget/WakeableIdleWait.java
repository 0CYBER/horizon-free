package dev.horizonlab.budget;

import java.util.concurrent.locks.LockSupport;

/** A bounded idle park, preserving Thread.sleep's InterruptedException/cleared-flag contract. */
public final class WakeableIdleWait {
    private WakeableIdleWait() { }

    public static void pause(Object blocker, long nanoseconds) throws InterruptedException {
        if (nanoseconds < 0) throw new IllegalArgumentException("Negative idle wait");
        if (Thread.interrupted()) throw new InterruptedException();
        LockSupport.parkNanos(blocker, nanoseconds);
        if (Thread.interrupted()) throw new InterruptedException();
    }
}
