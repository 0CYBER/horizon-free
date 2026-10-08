package dev.horizonlab.dispatch;

import com.seibel.distanthorizons.core.render.QuadTree.LodRenderSection;
import com.seibel.distanthorizons.core.render.QuadTree.QuadTreeTickNodeHolder;
import com.seibel.distanthorizons.core.util.objects.quadTree.QuadNode;
import dev.horizonlab.mixin.HorizonMixinPlugin;
import java.lang.management.ManagementFactory;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.Random;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;

/** Private Fabric fixture: semantic transitions and warmed allocation of the actual upstream holder. */
public final class TickDedupIntegration implements ClientModInitializer {
    private static volatile long checksum;
    @Override public void onInitializeClient() {
        ClientLifecycleEvents.CLIENT_STARTED.register(client -> {
            exercise();
            if (Boolean.getBoolean("horizon.fixture.exit")) System.exit(0);
        });
    }

    private static void exercise() {
        ArrayList<QuadNode<LodRenderSection>> nodes = new ArrayList<>();
        for (int index = 0; index < 512; index++) nodes.add(new QuadNode<>(index, (byte) 6));
        var holder = new QuadTreeTickNodeHolder();
        var present = new HashSet<QuadNode<LodRenderSection>>();
        var enabled = new HashSet<QuadNode<LodRenderSection>>();
        var disabled = new HashSet<QuadNode<LodRenderSection>>();
        var deleteChildren = new ArrayList<QuadNode<LodRenderSection>>();
        var random = new Random(0x4835);
        for (int step = 0; step < 6000; step++) {
            var node = step % 13 == 0 ? null : nodes.get(random.nextInt(nodes.size()));
            switch (random.nextInt(5)) {
                case 0 -> { holder.addEnableNode(node); if (present.add(node)) enabled.add(node); }
                case 1 -> { holder.addDisableNode(node); if (present.add(node)) disabled.add(node); }
                case 2 -> { holder.addEnableDeleteChildrenNode(node); if (present.add(node)) deleteChildren.add(node); }
                case 3 -> {
                    holder.removeEnableAndDisableNode(node);
                    enabled.remove(node); deleteChildren.remove(node); present.add(node); disabled.add(node);
                }
                case 4 -> { holder.clear(); present.clear(); enabled.clear(); disabled.clear(); deleteChildren.clear(); }
            }
            // Comparing iteration order as well as membership verifies that published collections stay unchanged.
            if (!new ArrayList<>(holder.getEnabledNodes()).equals(new ArrayList<>(enabled))
                    || !new ArrayList<>(holder.getDisableNodes()).equals(new ArrayList<>(disabled))
                    || !holder.getEnableDeleteChildrenNodes().equals(deleteChildren))
                throw new AssertionError("Action precedence or iteration order changed at step " + step);
        }
        var bean = (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();
        bean.setThreadAllocatedMemoryEnabled(true);
        bean.setThreadCpuTimeEnabled(true);
        measureLoop(holder, nodes, 5000);
        long id = Thread.currentThread().threadId(), bytes = bean.getThreadAllocatedBytes(id);
        long cpu = bean.getCurrentThreadCpuTime(), wall = System.nanoTime();
        measureLoop(holder, nodes, 20000);
        long wallDelta = System.nanoTime() - wall, cpuDelta = bean.getCurrentThreadCpuTime() - cpu;
        long byteDelta = bean.getThreadAllocatedBytes(id) - bytes;
        // clear must release references in the replacement too, not only empty the published collections.
        holder.clear();
        if (HorizonMixinPlugin.tickDedupEnabled()) {
            try {
                var field = holder.getClass().getDeclaredField("horizon$presentNodes"); field.setAccessible(true);
                if (!((java.util.Set<?>) field.get(holder)).isEmpty()) throw new AssertionError("Retained tick members");
            } catch (ReflectiveOperationException failure) { throw new AssertionError("Missing applied membership hook", failure); }
        }
        System.out.println("HORIZON_TICK_DEDUP_RESULT {\"enabled\":" + HorizonMixinPlugin.tickDedupEnabled()
            + ",\"allocated_bytes\":" + byteDelta + ",\"cpu_ns\":" + cpuDelta + ",\"wall_ns\":" + wallDelta
            + ",\"nodes_per_tick\":512,\"measured_ticks\":20000,\"verified_transitions\":6000,\"checksum\":" + checksum + "}");
    }

    private static void measureLoop(QuadTreeTickNodeHolder holder, ArrayList<QuadNode<LodRenderSection>> nodes, int repeats) {
        long sum = 0;
        for (int pass = 0; pass < repeats; pass++) {
            holder.clear();
            for (int index = 0; index < nodes.size(); index++) {
                var node = nodes.get(index);
                holder.addEnableDeleteChildrenNode(node);
                holder.addEnableDeleteChildrenNode(node);
            }
            if (holder.getEnableDeleteChildrenNodes().size() != nodes.size()) throw new AssertionError("Duplicate or lost node");
            sum += holder.getEnableDeleteChildrenNodes().size();
        }
        checksum = sum;
    }
}
