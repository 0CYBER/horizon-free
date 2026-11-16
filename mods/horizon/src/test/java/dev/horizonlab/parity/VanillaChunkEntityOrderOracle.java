package dev.horizonlab.parity;

import com.google.common.collect.ImmutableList;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.Marker;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.entity.ChunkEntities;
import net.minecraft.world.level.entity.EntityPersistentStorage;
import net.minecraft.world.level.entity.LevelCallback;
import net.minecraft.world.level.entity.PersistentEntitySectionManager;
import net.minecraft.world.level.entity.Visibility;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/**
 * The real PersistentEntitySectionManager and its EntityLookup: chunk loads,
 * an unload that stores the chunk (storeChunkSections), its reload from what
 * was stored, and a round trip through another level. Entities are named by
 * the last int of their UUID, as the engine test names its saved records.
 */
public final class VanillaChunkEntityOrderOracle {
    static Object unsafe;
    static Method allocate;
    static int nextId = 1000;
    /** n -> [x, y, z, passenger n or 0] of the engine test's records. */
    static final Map<Integer, double[]> RECORDS = new HashMap<>();

    static void field(Entity entity, String name, Object value) throws Exception {
        var field = Entity.class.getDeclaredField(name);
        field.setAccessible(true);
        field.set(entity, value);
    }

    /** A loaded entity: a fresh id from the counter, as every load constructs one. */
    static Marker entity(int n) throws Exception {
        double[] at = RECORDS.get(n);
        var entity = (Marker) allocate.invoke(unsafe, Marker.class);
        field(entity, "id", nextId++);
        field(entity, "uuid", new UUID(2703L << 32, n));
        field(entity, "position", new Vec3(at[0], at[1], at[2]));
        field(entity, "blockPosition", BlockPos.containing(at[0], at[1], at[2]));
        field(entity, "bb", new AABB(at[0] - 0.5, at[1], at[2] - 0.5, at[0] + 0.5, at[1] + 1, at[2] + 0.5));
        field(entity, "passengers", ImmutableList.of());
        return entity;
    }

    static int n(Entity entity) {
        return (int) entity.getUUID().getLeastSignificantBits();
    }

    /**
     * EntityType.loadEntitiesRecursive: each saved root is handed on before its
     * Passengers are loaded (loadEntityRecursive's postLoad, then
     * loadPassengersRecursive), so the stream is the tree in pre-order.
     */
    static void load(int n, List<Entity> out) throws Exception {
        var entity = entity(n);
        out.add(entity);
        int rider = (int) RECORDS.get(n)[3];
        if (rider != 0) {
            int at = out.size();
            load(rider, out);
            var passenger = out.get(at);
            field(passenger, "vehicle", entity);
            field(entity, "passengers", ImmutableList.of(passenger));
        }
    }

    static final class Storage implements EntityPersistentStorage<Entity> {
        final Map<ChunkPos, List<Integer>> roots = new HashMap<>();
        public CompletableFuture<ChunkEntities<Entity>> loadEntities(ChunkPos pos) {
            var out = new ArrayList<Entity>();
            try {
                for (int n : roots.getOrDefault(pos, List.of())) load(n, out);
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
            return CompletableFuture.completedFuture(new ChunkEntities<>(pos, out));
        }
        public void storeEntities(ChunkEntities<Entity> chunk) {
            var saved = new ArrayList<Integer>();
            chunk.getEntities().forEach(e -> saved.add(n(e)));
            roots.put(chunk.getPos(), saved);
            var line = new ArrayList<Integer>(List.of(chunk.getPos().x(), chunk.getPos().z()));
            line.addAll(saved);
            System.out.println("saved-order: " + line);
        }
        public void flush(boolean sync) {}
    }

    static final class Callbacks implements LevelCallback<Entity> {
        public void onCreated(Entity entity) {}
        public void onDestroyed(Entity entity) {}
        public void onTickingStart(Entity entity) {}
        public void onTickingEnd(Entity entity) {}
        public void onTrackingStart(Entity entity) {}
        public void onTrackingEnd(Entity entity) {}
        public void onSectionChange(Entity entity) {}
    }

    static void snapshot(PersistentEntitySectionManager<Entity> level) {
        var order = new ArrayList<Integer>();
        level.getEntityGetter().getAll().forEach(e -> order.add(n(e)));
        System.out.println("load-order: " + order);
    }

    static Entity find(PersistentEntitySectionManager<Entity> level, int n) {
        for (var e : level.getEntityGetter().getAll()) if (n(e) == n) return e;
        throw new AssertionError("no entity " + n);
    }

    static void record(int n, double x, double y, double z, int rider) {
        RECORDS.put(n, new double[] {x, y, z, rider});
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var allocator = Class.forName("sun.misc.Unsafe");
        var singleton = allocator.getDeclaredField("theUnsafe");
        singleton.setAccessible(true);
        unsafe = singleton.get(null);
        allocate = allocator.getMethod("allocateInstance", Class.class);
        // The engine test's chunk (0, 0) records, in saved order, across
        // storage families and three sections: 4, 0 and -2.
        record(1, 2.5, 70, 2.5, 0);
        record(2, 3.5, 70, 3.5, 0);
        record(3, 4.5, 70, 4.5, 0);
        record(4, 5.5, 70, 5.5, 5);
        record(5, 5.5, 70, 5.5, 0);
        record(6, 6.5, 10, 6.5, 0);
        record(7, 7.5, 70, 7.5, 0);
        record(8, 8.5, -20, 8.5, 0);
        // Chunk (1, 0).
        record(9, 18.5, 70, 2.5, 0);
        record(10, 19.5, 70, 2.5, 0);
        record(11, 20.5, 70, 2.5, 0);
        record(12, 21.5, 70, 2.5, 0);
        record(13, 22.5, 70, 2.5, 0);
        var a = new ChunkPos(0, 0);
        var b = new ChunkPos(1, 0);
        var store = new Storage();
        store.roots.put(a, List.of(1, 2, 3, 4, 6, 7, 8));
        store.roots.put(b, List.of(9, 10, 13, 11, 12));
        var overworld = new PersistentEntitySectionManager<>(Entity.class, new Callbacks(), store);
        var nether = new PersistentEntitySectionManager<>(Entity.class, new Callbacks(), new Storage());
        overworld.updateChunkStatus(a, Visibility.TICKING);
        overworld.updateChunkStatus(b, Visibility.TICKING);
        overworld.processPendingLoads();
        snapshot(overworld);
        // The chunk unloads: its roots are stored, then it leaves the lookup.
        overworld.updateChunkStatus(a, Visibility.HIDDEN);
        overworld.tick();
        snapshot(overworld);
        // It loads again from what was stored.
        overworld.updateChunkStatus(a, Visibility.TICKING);
        overworld.processPendingLoads();
        snapshot(overworld);
        // Entity 2 changes dimension and comes back: Entity.teleport's
        // removeAfterChangingDimensions, then a new entity added to the
        // destination (ServerLevel.addDuringTeleport -> addNewEntity).
        nether.updateChunkStatus(a, Visibility.TICKING);
        nether.processPendingLoads();
        find(overworld, 2).setRemoved(Entity.RemovalReason.CHANGED_DIMENSION);
        if (!nether.addNewEntity(entity(2))) throw new AssertionError("nether add");
        snapshot(overworld);
        find(nether, 2).setRemoved(Entity.RemovalReason.CHANGED_DIMENSION);
        if (!overworld.addNewEntity(entity(2))) throw new AssertionError("overworld add");
        snapshot(overworld);
        // The level saves: the chunk's roots in section, then lookup, order.
        overworld.autoSave();
    }
}
