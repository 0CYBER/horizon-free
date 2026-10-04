package dev.horizonlab.dispatch;

import com.seibel.distanthorizons.core.dataObjects.fullData.sources.FullDataSourceV2;
import com.seibel.distanthorizons.core.pos.DhSectionPos;
import com.seibel.distanthorizons.core.util.FullDataPointUtil;
import dev.horizonlab.mixin.HorizonMixinPlugin;
import it.unimi.dsi.fastutil.longs.LongArrayList;
import java.lang.management.ManagementFactory;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.Random;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;

/** Private applied-hook fixture, using generated column data and the actual public lookup. */
public final class PositionIntegration implements ClientModInitializer {
    private static volatile long checksum;

    @Override public void onInitializeClient() {
        ClientLifecycleEvents.CLIENT_STARTED.register(client -> {
            try { exercise(); }
            catch (Exception failure) { throw new AssertionError("Terrain position fixture failed", failure); }
            if (Boolean.getBoolean("horizon.fixture.exit")) System.exit(0);
        });
    }

    private static FullDataSourceV2 source(byte level, int x, int z) throws Exception {
        var owner = FullDataSourceV2.createEmpty(DhSectionPos.encode(level, x, z));
        for (int cx = 0; cx < 64; cx++) for (int cz = 0; cz < 64; cz++) {
            var column = new LongArrayList();
            for (int layer = 2; layer >= 0; layer--) {
                long point = FullDataPointUtil.encode(1 + ((cx * 7 + cz * 11 + layer) & 255), 32,
                    layer * 32, (byte) 15, (byte) (layer * 3));
                if (FullDataPointUtil.getHeight(point) != 32 || FullDataPointUtil.getBottomY(point) != layer * 32)
                    throw new AssertionError("Generated publisher data has wrong height");
                column.add(point);
            }
            owner.dataPoints[FullDataSourceV2.relativePosToIndex(cx, cz)] = column;
        }
        return owner;
    }

    private static long dataFingerprint(FullDataSourceV2 owner) {
        long hash = owner.getPos();
        for (var column : owner.dataPoints) {
            hash = hash * 31 + column.size();
            for (int index = 0; index < column.size(); index++) hash = hash * 31 + column.getLong(index);
        }
        return hash;
    }

    private static void exercise() throws Exception {
        boolean enabled = HorizonMixinPlugin.terrainPositionEnabled();
        boolean applied = java.util.Arrays.stream(FullDataSourceV2.class.getDeclaredMethods())
            .anyMatch(method -> method.getName().contains("horizon$relativePosition"));
        if (applied != enabled) throw new AssertionError("Lookup redirect does not match requested state");
        var digest = MessageDigest.getInstance("SHA-256");
        byte[] bytes = new byte[8];
        var random = new Random(0x49a91);
        int queries = 0, nonzero = 0, owners = 0;
        long fingerprints = 0;
        for (byte level : new byte[] {6, 7, 8, 10, 14, 20}) {
            for (int[] origin : new int[][] {{-1, -1}, {-7, 9}, {0, 0}, {13, -21}, {11, -13}}) {
                var owner = source(level, origin[0], origin[1]);
                long before = dataFingerprint(owner);
                int minX = DhSectionPos.getMinCornerBlockX(owner.getPos());
                int minZ = DhSectionPos.getMinCornerBlockZ(owner.getPos());
                int width = DhSectionPos.getDetailLevelWidthInBlocks(level);
                for (int index = 0; index < 16384; index++) {
                    int x = minX + random.nextInt(width), z = minZ + random.nextInt(width);
                    if ((index & 1023) == 0) x = minX - 1;
                    if ((index & 1023) == 1) z = minZ + width;
                    int y = random.nextInt(144) - 80;
                    long value = owner.getDataPointAtBlockPos(x, y, z, -64);
                    if (value != 0) nonzero++;
                    for (int at = 0; at < 8; at++) bytes[at] = (byte) (value >>> (56 - 8 * at));
                    digest.update(bytes); queries++;
                }
                if (before != dataFingerprint(owner)) throw new AssertionError("Lookup mutated source data");
                fingerprints = fingerprints * 31 + before; owners++;
            }
        }
        if (nonzero < 10000) throw new AssertionError("Fixture did not exercise populated columns");
        var owner = source((byte) 10, -13, -7);
        int[] xs = new int[256], zs = new int[256];
        int minX = DhSectionPos.getMinCornerBlockX(owner.getPos()), minZ = DhSectionPos.getMinCornerBlockZ(owner.getPos());
        for (int index = 0; index < 256; index++) { xs[index] = minX + index * 3; zs[index] = minZ + (index * 37 & 1023); }
        loop(owner, xs, zs, 10000000);
        Thread.sleep(1000);
        var cpu = ManagementFactory.getThreadMXBean();
        var allocations = (com.sun.management.ThreadMXBean) cpu;
        if (allocations.isThreadAllocatedMemorySupported() && !allocations.isThreadAllocatedMemoryEnabled())
            allocations.setThreadAllocatedMemoryEnabled(true);
        long tid = Thread.currentThread().threadId();
        long wall = 0, cpuTime = 0, allocated = 0;
        StringBuilder observations = new StringBuilder("[");
        for (int repeat = 0; repeat < 3; repeat++) {
            loop(owner, xs, zs, 4000000);
            Thread.sleep(200);
            long allocationStart = allocations.getThreadAllocatedBytes(tid), cpuStart = cpu.getCurrentThreadCpuTime(), start = System.nanoTime();
            loop(owner, xs, zs, 2000000);
            long observedWall = System.nanoTime() - start, observedCpu = cpu.getCurrentThreadCpuTime() - cpuStart;
            long observedAllocation = allocations.getThreadAllocatedBytes(tid) - allocationStart;
            wall += observedWall; cpuTime += observedCpu; allocated += observedAllocation;
            if (repeat != 0) observations.append(',');
            observations.append("{\"cpu_ns\":").append(observedCpu).append(",\"wall_ns\":").append(observedWall)
                .append(",\"allocated_bytes\":").append(observedAllocation).append('}');
        }
        observations.append(']');
        System.out.println("HORIZON_POSITION_RESULT {\"enabled\":" + enabled + ",\"applied_hook_checked\":true,\"owners\":" + owners
            + ",\"queries\":" + queries + ",\"nonzero\":" + nonzero + ",\"lookup_digest\":\"" + HexFormat.of().formatHex(digest.digest())
            + "\",\"data_fingerprint\":" + fingerprints + ",\"data_unchanged\":true,\"measured_checks\":6000000,\"checksum\":" + checksum
            + ",\"allocated_bytes\":" + allocated + ",\"cpu_ns\":" + cpuTime + ",\"wall_ns\":" + wall + ",\"observations\":" + observations + "}");
    }

    private static void loop(FullDataSourceV2 owner, int[] xs, int[] zs, int count) {
        long sum = 0;
        for (int index = 0; index < count; index++) sum += owner.getDataPointAtBlockPos(xs[index & 255], -48, zs[index & 255], -64);
        checksum = sum;
    }
}
