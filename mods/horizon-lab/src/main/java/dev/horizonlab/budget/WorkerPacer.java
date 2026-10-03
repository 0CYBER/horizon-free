package dev.horizonlab.budget;

/** Per-worker primitive state; amortizes short waits without discarding fractional milliseconds. */
public final class WorkerPacer {
    public static final long QUANTUM_NS = 250_000;
    public static final long MAX_CREDIT_NS = 50_000_000;
    private long started;
    private long completed;
    private double debtNs;
    private double previousRatio = 1;

    public void begin(long now) {
        if (completed != 0 && now - completed > 1_000_000_000L) debtNs = 0;
        started = now;
    }

    public long finish(long now, double ratio) {
        long workNs = Math.max(0, now - started);
        completed = now;
        if (ratio != previousRatio) debtNs = 0;
        previousRatio = ratio;
        if (!Double.isFinite(ratio) || ratio <= 0 || ratio >= 1) {
            debtNs = 0;
            return 0;
        }
        debtNs += workNs * (1.0 / ratio - 1.0);
        return debtNs < QUANTUM_NS ? 0 : (long) Math.min(debtNs, Long.MAX_VALUE / 4.0);
    }

    public void pauseCompleted(long now) {
        debtNs = Math.max(-MAX_CREDIT_NS, debtNs - Math.max(0, now - completed));
        completed = now;
    }

    public void interrupted(long now) { debtNs = 0; completed = now; }
}
