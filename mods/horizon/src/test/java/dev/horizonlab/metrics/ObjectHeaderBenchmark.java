package dev.horizonlab.metrics;

import com.seibel.distanthorizons.core.pos.DhChunkPos;
import com.sun.management.HotSpotDiagnosticMXBean;
import java.lang.management.ManagementFactory;
import java.util.Locale;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/** Actual publisher types in an isolated JVM; allocated bytes are not retained game RAM. */
public final class ObjectHeaderBenchmark {
    private static volatile Object[] retained;
    private static final int COUNT = 100_000;

    private static void measure(int kind, boolean report) {
        Object[] values = new Object[COUNT]; // Reference storage excluded from the counter.
        var threads = (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();
        if (!threads.isThreadAllocatedMemorySupported())
            throw new IllegalStateException("Thread allocation accounting unavailable");
        threads.setThreadAllocatedMemoryEnabled(true);
        long id = Thread.currentThread().threadId();
        long start = threads.getThreadAllocatedBytes(id);
        for (int i = 0; i < COUNT; i++) values[i] = switch (kind) {
            case 0 -> new AABB(i, 0, 0, i + 1, 1, 1);
            case 1 -> new Vec3(i, 2, 3);
            case 2 -> new DhChunkPos(i, 0);
            case 3 -> Long.valueOf(i + 1000L); // Outside the standard boxed-value cache.
            default -> new Object();
        };
        retained = values; // Escaping objects must actually be materialized.
        long bytes = threads.getThreadAllocatedBytes(id) - start;
        if (bytes <= 0) throw new AssertionError("Missing allocation counter");
        for (int i = 0; i < COUNT; i++) {
            boolean valid = switch (kind) {
                case 0 -> ((AABB) values[i]).minX == i && ((AABB) values[i]).maxX == i + 1;
                case 1 -> ((Vec3) values[i]).x == i && ((Vec3) values[i]).y == 2 && ((Vec3) values[i]).z == 3;
                case 2 -> values[i].equals(new DhChunkPos(i, 0));
                case 3 -> ((Long) values[i]).longValue() == i + 1000L;
                default -> values[i] != null && (i == 0 || values[i] != values[i - 1]);
            };
            if (!valid) throw new AssertionError("Object values changed");
        }
        if (report) System.out.printf(Locale.ROOT,
            "type=%s objects=%d allocated_bytes=%d bytes_per_object=%.3f%n",
            new String[]{"AABB", "Vec3", "DhChunkPos", "Long", "Object"}[kind], COUNT, bytes, (double) bytes / COUNT);
    }

    public static void main(String[] args) {
        var hotspot = ManagementFactory.getPlatformMXBean(HotSpotDiagnosticMXBean.class);
        boolean enabled = Boolean.parseBoolean(hotspot.getVMOption("UseCompactObjectHeaders").getValue());
        if (args.length != 1 || enabled != Boolean.parseBoolean(args[0]))
            throw new AssertionError("JVM compact header setting does not match the requested mode");
        System.out.println("compact_object_headers=" + enabled);
        for (int kind = 0; kind < 5; kind++) {
            for (int warmup = 0; warmup < 3; warmup++) measure(kind, false);
            for (int trial = 0; trial < 3; trial++) measure(kind, true);
        }
    }
}
