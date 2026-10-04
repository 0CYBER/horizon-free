package dev.horizonlab.dispatch;

import dev.horizonlab.mixin.HorizonMixinPlugin;
import dev.tr7zw.entityculling.EntityCullingModBase;
import dev.tr7zw.entityculling.access.Cullable;
import java.lang.management.ManagementFactory;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.Iterator;
import java.util.function.Function;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ChestBlockEntity;

/** Private fixture validates applied redirects and live handler registration on the publisher's instance. */
public final class CullingEmptyIntegration implements ClientModInitializer {
    private static volatile boolean result;
    private static final class CountingSet<E> extends HashSet<E> {
        int iterators;
        @Override public Iterator<E> iterator() { iterators++; return super.iterator(); }
    }
    @Override public void onInitializeClient() {
        ClientLifecycleEvents.CLIENT_STARTED.register(client -> {
            try { exercise(); }
            catch (ReflectiveOperationException failure) { throw new AssertionError("Culling integration failed", failure); }
            if (Boolean.getBoolean("horizon.fixture.exit")) System.exit(0);
        });
    }

    private static void exercise() throws ReflectiveOperationException {
        var owner = EntityCullingModBase.instance;
        if (owner == null) throw new AssertionError("Publisher culling mod was not initialized");
        Field entity = EntityCullingModBase.class.getDeclaredField("dynamicEntityWhitelist");
        Field block = EntityCullingModBase.class.getDeclaredField("dynamicBlockEntityWhitelist");
        entity.setAccessible(true); block.setAccessible(true);
        Object oldEntity = entity.get(owner), oldBlock = block.get(owner);
        boolean enabled = HorizonMixinPlugin.cullingEmptyEnabled();
        var entities = new CountingSet<Function<net.minecraft.world.entity.Entity, Boolean>>();
        var blocks = new CountingSet<Function<BlockEntity, Boolean>>();
        var chest = new ChestBlockEntity(BlockPos.ZERO, Blocks.CHEST.defaultBlockState());
        var subject = (Cullable) (Object) chest;
        try {
            entity.set(owner, entities); block.set(owner, blocks);
            if (owner.isEntityDynamicWhitelisted(null) || owner.isBlockEntityDynamicWhitelisted(subject))
                throw new AssertionError("Empty whitelist unexpectedly matched");
            int expectedEmpty = enabled ? 0 : 1;
            if (entities.iterators != expectedEmpty || blocks.iterators != expectedEmpty)
                throw new AssertionError("Empty whitelist iterator hook did not apply");
            var called = new ArrayList<Integer>();
            Function<BlockEntity, Boolean> first = value -> { called.add(1); return false; };
            Function<BlockEntity, Boolean> second = value -> { called.add(2); return value == chest; };
            owner.addDynamicBlockEntityWhitelist(first);
            owner.addDynamicBlockEntityWhitelist(second);
            owner.addDynamicBlockEntityWhitelist(first); // Duplicate registration remains a set operation.
            var expected = new ArrayList<Integer>();
            for (var handler : blocks) { expected.add(handler == first ? 1 : 2); if (handler == second) break; }
            int before = blocks.iterators;
            if (!owner.isBlockEntityDynamicWhitelisted(subject) || !called.equals(expected) || blocks.iterators != before + 1)
                throw new AssertionError("Live registration, callback order or short circuit changed");
            int callbackCount = called.size();
            if (owner.isBlockEntityDynamicWhitelisted(null) || called.size() != callbackCount)
                throw new AssertionError("Null unexpectedly matched or invoked a callback");
            owner.addDynamicEntityWhitelist(value -> { throw new AssertionError("Wrong entity type invoked handler"); });
            before = entities.iterators;
            if (owner.isEntityDynamicWhitelisted(subject) || entities.iterators != before + 1)
                throw new AssertionError("Nonempty entity iterator or type guard changed");
            blocks.clear(); entities.clear(); before = blocks.iterators;
            if (owner.isBlockEntityDynamicWhitelisted(subject) || blocks.iterators != before + expectedEmpty)
                throw new AssertionError("Empty state after clear was cached incorrectly");
            // Normal HashSets for the allocation observation; instrumentation is not in this loop.
            entity.set(owner, new HashSet<>()); block.set(owner, new HashSet<>());
            loop(owner, 200000);
            var bean = (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();
            bean.setThreadAllocatedMemoryEnabled(true); bean.setThreadCpuTimeEnabled(true);
            long id = Thread.currentThread().threadId(), bytes = bean.getThreadAllocatedBytes(id);
            long cpu = bean.getCurrentThreadCpuTime(), wall = System.nanoTime();
            loop(owner, 2000000);
            long wallDelta = System.nanoTime() - wall, cpuDelta = bean.getCurrentThreadCpuTime() - cpu;
            long byteDelta = bean.getThreadAllocatedBytes(id) - bytes;
            System.out.println("HORIZON_CULLING_EMPTY_RESULT {\"enabled\":" + enabled
                + ",\"empty_iterator_calls\":" + expectedEmpty + ",\"live_registration_checked\":true"
                + ",\"block_callback_order_checked\":true,\"type_guard_checked\":true"
                + ",\"allocated_bytes\":" + byteDelta + ",\"cpu_ns\":" + cpuDelta + ",\"wall_ns\":" + wallDelta
                + ",\"measured_calls\":4000000,\"result\":" + result + "}");
        } finally { entity.set(owner, oldEntity); block.set(owner, oldBlock); }
    }

    private static void loop(EntityCullingModBase owner, int iterations) {
        boolean found = false;
        for (int index = 0; index < iterations; index++) {
            found |= owner.isEntityDynamicWhitelisted(null);
            found |= owner.isBlockEntityDynamicWhitelisted(null);
        }
        result = found;
    }
}
