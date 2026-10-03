package dev.horizonlab.metrics;

import java.util.Arrays;

/** Original bounded measurement utilities; no Minecraft dependency. */
public record SampleStats(int count, double meanMs, double p50Ms, double p95Ms,
                          double p99Ms, double maxMs, double meanRate, double lowOnePercentRate,
                          int over16Ms, int over33Ms, int over50Ms, int over100Ms) {
    public static SampleStats of(long[] samples, int count) {
        if (count < 0 || count > samples.length) throw new IllegalArgumentException("Invalid sample count");
        if (count == 0) return new SampleStats(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
        long[] sorted = Arrays.copyOf(samples, count);
        double sum = 0;
        int over16 = 0, over33 = 0, over50 = 0, over100 = 0;
        for (long ns : sorted) {
            if (ns <= 0) throw new IllegalArgumentException("Sample duration must be positive");
            sum += ns;
            if (ns > 16_666_667L) over16++;
            if (ns > 33_333_333L) over33++;
            if (ns > 50_000_000L) over50++;
            if (ns > 100_000_000L) over100++;
        }
        Arrays.sort(sorted);
        int tail = Math.max(1, (int) Math.ceil(count * 0.01));
        double tailSum = 0;
        for (int i = count - tail; i < count; i++) tailSum += sorted[i];
        return new SampleStats(count, sum / count / 1e6, quantile(sorted, 0.50) / 1e6,
            quantile(sorted, 0.95) / 1e6, quantile(sorted, 0.99) / 1e6,
            sorted[count - 1] / 1e6, 1e9 * count / sum, 1e9 * tail / tailSum,
            over16, over33, over50, over100);
    }

    private static long quantile(long[] sorted, double fraction) {
        return sorted[Math.max(0, (int) Math.ceil(sorted.length * fraction) - 1)];
    }
}
