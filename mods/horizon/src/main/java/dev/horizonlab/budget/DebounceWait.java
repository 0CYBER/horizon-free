package dev.horizonlab.budget;

/** Wait only while an unchanged debounce deadline blocks progress. */
public final class DebounceWait {
    public static final long MAX_WAIT_NS = 1_000_000L;
    private DebounceWait() {}

    public static long delay(long now, long firstQueued, long debounce, boolean preprocessingPending) {
        if (preprocessingPending || debounce <= 0) return 0;
        long age = now - firstQueued;
        if (age < 0 || age >= debounce) return 0;
        return Math.min(MAX_WAIT_NS, debounce - age);
    }
}
