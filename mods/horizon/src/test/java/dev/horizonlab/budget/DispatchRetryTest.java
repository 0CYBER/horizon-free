package dev.horizonlab.budget;

public final class DispatchRetryTest {
    private static void check(boolean condition) { if (!condition) throw new AssertionError(); }
    public static void main(String[] args) {
        DispatchRetry policy = new DispatchRetry();
        policy.beginPass();
        policy.beginExecutor();
        check(policy.visibleSlots(0) == 0 && policy.visibleSlots(3) == 3);
        check(!policy.needsRetry(false, false) && policy.needsRetry(true, false));
        check(policy.retryMilliseconds(1000) == 1000);
        policy.rejected();
        check(policy.visibleSlots(0) == Integer.MAX_VALUE && policy.needsRetry(false, false));
        check(policy.retryMilliseconds(1000) == 1);
        policy.beginExecutor();
        check(policy.visibleSlots(1) == 1); // Another executor can consume a real free slot.
        check(policy.needsRetry(false, false)); // Still retry the earlier rejected executor.
        check(!policy.needsRetry(false, true) && !policy.needsRetry(true, true));
        policy.beginPass();
        check(!policy.needsRetry(false, false) && policy.retryMilliseconds(1000) == 1000);
        System.out.println("Dispatch retry policy tests passed.");
    }
}
