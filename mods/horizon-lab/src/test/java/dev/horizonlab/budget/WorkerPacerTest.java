package dev.horizonlab.budget;

public final class WorkerPacerTest {
    private static void check(boolean value, String reason) { if (!value) throw new AssertionError(reason); }

    public static void main(String[] args) {
        for (double ratio : new double[]{0.01, 0.05, 0.1, 0.5, 0.9, 1}) {
            for (long work : new long[]{100, 10_000, 1_000_000, 100_000_000}) {
                for (long overshoot : new long[]{80_000, 1_000_000, 15_000_000}) {
                    WorkerPacer pacer = new WorkerPacer();
                    long now = 1;
                    long active = 0;
                    long slept = 0;
                    for (int i = 0; i < 1000; i++) {
                        pacer.begin(now);
                        now += work;
                        active += work;
                        long delay = pacer.finish(now, ratio);
                        check(delay == 0 || delay >= WorkerPacer.QUANTUM_NS, "Small waits are batched");
                        if (delay != 0) {
                            // Simulate timer overshoot; future tasks must receive credit for it.
                            delay += overshoot;
                            now += delay;
                            slept += delay;
                            pacer.pauseCompleted(now);
                        }
                    }
                    double expectedSleep = active * (1.0 / ratio - 1);
                    check(Math.abs(slept - expectedSleep) <= WorkerPacer.QUANTUM_NS + overshoot + 1,
                        "Duty cycle retained across job sizes and timer overshoot: " + ratio + "/" + work);
                }
            }
        }
        WorkerPacer pacer = new WorkerPacer();
        pacer.begin(1); pacer.finish(10_001, 0.5);
        pacer.begin(20_001);
        check(pacer.finish(30_001, 1) == 0, "A full-duty configuration discards old delay debt");
        for (double invalid : new double[]{0, -1, Double.NaN, Double.POSITIVE_INFINITY}) {
            pacer.begin(40_001);
            check(pacer.finish(50_001, invalid) == 0, "Invalid ratios cannot cause unbounded waits");
        }
        pacer.begin(60_001); pacer.finish(70_001, 0.5);
        pacer.begin(2_000_070_001L);
        check(pacer.finish(2_000_080_001L, 0.5) == 0, "Idle workers discard stale delay debt");
        pacer.interrupted(2_000_080_001L);
        System.out.println("Worker pacing tests passed");
    }
}
