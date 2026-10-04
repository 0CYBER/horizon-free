package dev.horizonlab.dispatch;

import com.logisticscraft.occlusionculling.DataProvider;
import com.logisticscraft.occlusionculling.OcclusionCullingInstance;
import com.logisticscraft.occlusionculling.util.Vec3d;
import dev.horizonlab.mixin.HorizonMixinPlugin;
import dev.horizonlab.dispatch.fixture.CullingRayAccess;
import java.lang.management.ManagementFactory;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.Random;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;

/** Private applied-hook fixture; baseline and candidate run the same publisher geometry. */
public final class CullingRayIntegration implements ClientModInitializer {
    private static volatile long checksum;
    private static final class Provider implements DataProvider {
        final MessageDigest digest;
        final byte[] coordinates = new byte[12];
        final int pattern;
        int queries;
        Provider(int pattern) throws Exception { this.pattern = pattern; digest = MessageDigest.getInstance("SHA-256"); }
        private void number(int offset, int value) {
            for (int byteIndex = 0; byteIndex < 4; byteIndex++) coordinates[offset + byteIndex] = (byte) (value >>> (24 - 8 * byteIndex));
        }
        @Override public boolean prepareChunk(int x, int z) {
            number(0, x); number(4, z); number(8, 0x1122); digest.update(coordinates); return true;
        }
        @Override public boolean isOpaqueFullCube(int x, int y, int z) {
            number(0, x); number(4, y); number(8, z); digest.update(coordinates); queries++;
            return switch (pattern) {
                case 0 -> false;
                case 1 -> true;
                case 2 -> x == 0;
                case 3 -> y == 0;
                case 4 -> z == 0;
                case 5 -> ((x + y + z) & 1) == 0;
                case 6 -> x % 3 == 0 && z % 3 == 0;
                default -> Math.floorMod(x * 73856093 ^ y * 19349663 ^ z * 83492791, 7) < 2;
            };
        }
    }
    @Override public void onInitializeClient() {
        ClientLifecycleEvents.CLIENT_STARTED.register(client -> {
            try { exercise(); }
            catch (Exception failure) { throw new AssertionError("Culling ray fixture failed", failure); }
            if (Boolean.getBoolean("horizon.fixture.exit")) System.exit(0);
        });
    }
    private static void bit(MessageDigest digest, boolean value) { digest.update((byte) (value ? 1 : 0)); }
    private static Vec3d scratch(OcclusionCullingInstance owner, String fieldName) throws Exception {
        var field = owner.getClass().getDeclaredField(fieldName); field.setAccessible(true); return (Vec3d) field.get(owner);
    }
    private static void same(Vec3d actual, Vec3d expected) {
        if (Double.doubleToLongBits(actual.x) != Double.doubleToLongBits(expected.x)
            || Double.doubleToLongBits(actual.y) != Double.doubleToLongBits(expected.y)
            || Double.doubleToLongBits(actual.z) != Double.doubleToLongBits(expected.z))
            throw new AssertionError("Scratch reset or upstream arithmetic changed: actual=" + actual.x + "," + actual.y + "," + actual.z
                + " expected=" + expected.x + "," + expected.y + "," + expected.z);
    }
    private static void exercise() throws Exception {
        boolean enabled = HorizonMixinPlugin.cullingRayScratchEnabled();
        var visibility = MessageDigest.getInstance("SHA-256");
        var queries = MessageDigest.getInstance("SHA-256");
        var intersections = MessageDigest.getInstance("SHA-256");
        var random = new Random(0x4a19);
        int boxes = 0, reads = 0;
        for (int pattern = 0; pattern < 8; pattern++) {
            var firstProvider = new Provider(pattern); var secondProvider = new Provider(pattern);
            var first = new OcclusionCullingInstance(16, firstProvider);
            var second = new OcclusionCullingInstance(16, secondProvider);
            if (enabled) {
                var direction = scratch(first, "horizon$rayDirection");
                var inverse = scratch(first, "horizon$inverseDirection");
                if (direction == inverse || direction == scratch(second, "horizon$rayDirection")
                    || inverse == scratch(second, "horizon$inverseDirection")) throw new AssertionError("Scratch state was shared");
            }
            Vec3d camera = new Vec3d(-2.5, 0.5, 2.5), otherCamera = new Vec3d(2.5, -0.5, -2.5);
            for (int index = 0; index < 800; index++) {
                if (index % 17 == 0) { first.resetCache(); second.resetCache(); }
                double x = random.nextInt(15) - 7 + (index % 3 == 0 ? 0 : random.nextDouble());
                double y = random.nextInt(15) - 7 + (index % 5 == 0 ? 0 : random.nextDouble());
                double z = random.nextInt(15) - 7 + (index % 7 == 0 ? 0 : random.nextDouble());
                var min = new Vec3d(x, y, z);
                var max = new Vec3d(x + (index % 11 == 0 ? 0 : random.nextDouble() * 2),
                    y + (index % 13 == 0 ? 0 : random.nextDouble() * 2), z + (index % 19 == 0 ? 0 : random.nextDouble() * 2));
                bit(visibility, first.isAABBVisible(min, max, camera));
                bit(visibility, second.isAABBVisible(min, max, otherCamera));
                boxes += 2;
            }
            queries.update(firstProvider.digest.digest()); queries.update(secondProvider.digest.digest());
            reads += firstProvider.queries + secondProvider.queries;
        }
        var owner = new OcclusionCullingInstance(16, new Provider(1));
        var access = (CullingRayAccess) owner;
        var camera = new Vec3d(-2, -2, -2);
        double[] values = {0.0, -0.0, 1, -1, Double.MIN_VALUE, -Double.MIN_VALUE, 1e-300, -1e-300,
                          1e300, -1e300, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, Double.NaN};
        int rays = 0;
        for (double x : values) for (double y : values) for (double z : values) {
            var input = new Vec3d(x, y, z);
            bit(intersections, access.horizon$intersects(new int[]{0, 0, 0}, camera, input));
            same(input, new Vec3d(x, y, z));
            if (enabled) same(scratch(owner, "horizon$inverseDirection"), new Vec3d(1, 1, 1).div(input));
            rays++;
        }
        var points = new Vec3d[64];
        for (int index = 0; index < points.length; index++) points[index] = new Vec3d(4 + index * 0.001, 4, 4);
        java.util.Arrays.fill(access.horizon$lastBlock(), 0);
        java.util.Arrays.fill(access.horizon$cameraBlock(), -2);
        var directionObject = enabled ? scratch(owner, "horizon$rayDirection") : null;
        var inverseObject = enabled ? scratch(owner, "horizon$inverseDirection") : null;
        for (var point : points) {
            access.horizon$checks(true);
            bit(intersections, access.horizon$visible(camera, new Vec3d[]{point}, 1));
            if (enabled) {
                var expected = new Vec3d(camera.x - point.x, camera.y - point.y, camera.z - point.z).normalize();
                same(directionObject, expected); same(inverseObject, new Vec3d(1, 1, 1).div(expected));
            }
        }
        loop(access, camera, points, 200000);
        var bean = (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();
        bean.setThreadAllocatedMemoryEnabled(true); bean.setThreadCpuTimeEnabled(true);
        long id = Thread.currentThread().threadId(), bytes = bean.getThreadAllocatedBytes(id);
        long cpu = bean.getCurrentThreadCpuTime(), wall = System.nanoTime();
        loop(access, camera, points, 2000000);
        long wallDelta = System.nanoTime() - wall, cpuDelta = bean.getCurrentThreadCpuTime() - cpu;
        long byteDelta = bean.getThreadAllocatedBytes(id) - bytes;
        if (enabled && (scratch(owner, "horizon$rayDirection") != directionObject
            || scratch(owner, "horizon$inverseDirection") != inverseObject)) throw new AssertionError("Scratch object was replaced");
        System.out.println("HORIZON_CULLING_RAYS_RESULT {\"enabled\":" + enabled + ",\"public_boxes\":" + boxes
            + ",\"provider_reads\":" + reads + ",\"edge_rays\":" + rays + ",\"visibility_digest\":\""
            + HexFormat.of().formatHex(visibility.digest()) + "\",\"query_digest\":\""
            + HexFormat.of().formatHex(queries.digest()) + "\",\"intersection_digest\":\""
            + HexFormat.of().formatHex(intersections.digest()) + "\",\"allocated_bytes\":" + byteDelta
            + ",\"cpu_ns\":" + cpuDelta + ",\"wall_ns\":" + wallDelta + ",\"measured_checks\":2000000"
            + ",\"checksum\":" + checksum + ",\"input_unchanged\":true,\"scratch_ownership_checked\":true}");
    }
    private static void loop(CullingRayAccess access, Vec3d camera, Vec3d[] points, int count) {
        var one = new Vec3d[1]; long sum = 0;
        for (int index = 0; index < count; index++) {
            one[0] = points[index & 63]; access.horizon$checks(true);
            if (access.horizon$visible(camera, one, 1)) sum++;
        }
        checksum = sum;
    }
}
