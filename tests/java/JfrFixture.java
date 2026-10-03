import java.nio.file.Path;
import java.time.Duration;
import jdk.jfr.Recording;

/** Produces real JDK counter/GC events for the reader's independent integration check. */
public final class JfrFixture {
    private static volatile Object[] retained;
    public static void main(String[] args) throws Exception {
        Thread.currentThread().setName("Horizon JFR fixture \"quoted\"\tthread");
        try (var recording = new Recording()) {
            recording.enable("jdk.ThreadAllocationStatistics").withPeriod(Duration.ofMillis(100));
            recording.enable("jdk.ExecutionSample").withPeriod(Duration.ofMillis(10));
            recording.enable("jdk.ObjectAllocationSample").withStackTrace();
            recording.enable("jdk.GarbageCollection");
            recording.enable("jdk.GCPhasePause");
            recording.enable("jdk.GCHeapSummary");
            recording.start();
            for (int batch = 0; batch < 12; batch++) {
                Object[] objects = new Object[100_000];
                for (int i = 0; i < objects.length; i++) objects[i] = new byte[32 + (i % 8)];
                retained = objects;
                Thread.sleep(120);
            }
            System.gc();
            Thread.sleep(250);
            recording.stop();
            recording.dump(Path.of(args[0]));
        }
    }
}
