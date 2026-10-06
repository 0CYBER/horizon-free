package dev.horizonlab.budget;

/** Per-picker state, used only while its existing dispatch lock is held. */
public final class DispatchRetry {
    private boolean rejectedExecutor;
    private boolean rejectedPass;

    public void beginPass() { rejectedExecutor = false; rejectedPass = false; }
    public void beginExecutor() { rejectedExecutor = false; }
    public void rejected() { rejectedExecutor = true; rejectedPass = true; }
    public int visibleSlots(int occupied) { return rejectedExecutor ? Integer.MAX_VALUE : occupied; }
    public boolean needsRetry(boolean original, boolean shutdown) { return !shutdown && (original || rejectedPass); }
    public long retryMilliseconds(long original) { return rejectedPass ? 1 : original; }
}
