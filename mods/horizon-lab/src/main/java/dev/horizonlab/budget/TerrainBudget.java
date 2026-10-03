package dev.horizonlab.budget;

/** Foreground-first background duty cycle. Inputs are two-second observation windows. */
public final class TerrainBudget {
    private double ratio;
    private int healthyWindows;

    public TerrainBudget(double ceiling) { ratio = ceiling; }

    public double update(double ceiling, double meanFrameMs, double targetFrameMs,
                         double lateFraction, double heapFraction) {
        if (!Double.isFinite(ceiling) || ceiling < 0.01 || ceiling > 1.0
                || !Double.isFinite(meanFrameMs) || meanFrameMs < 0
                || !Double.isFinite(targetFrameMs) || targetFrameMs <= 0
                || !Double.isFinite(lateFraction) || lateFraction < 0 || lateFraction > 1
                || !Double.isFinite(heapFraction) || heapFraction < 0 || heapFraction > 1) {
            throw new IllegalArgumentException("Invalid budget observations");
        }
        double floor = Math.min(ceiling, 0.05);
        ratio = Math.min(ratio, ceiling);
        boolean pressure = meanFrameMs > targetFrameMs * 1.35 || lateFraction > 0.20 || heapFraction > 0.85;
        boolean healthy = meanFrameMs < targetFrameMs * 1.10 && lateFraction < 0.05 && heapFraction < 0.75;
        if (pressure) {
            healthyWindows = 0;
            ratio = Math.max(floor, ratio * 0.65);
        } else if (healthy) {
            if (++healthyWindows >= 3) {
                ratio = Math.min(ceiling, ratio + 0.025);
                healthyWindows = 0;
            }
        } else {
            healthyWindows = 0;
        }
        return ratio;
    }

    public double ratio() { return ratio; }
}
