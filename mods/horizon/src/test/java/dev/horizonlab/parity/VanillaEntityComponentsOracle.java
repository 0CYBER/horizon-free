package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.Predicate;
import net.minecraft.advancements.predicates.entity.EntityPredicate;
import net.minecraft.core.*;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryOps;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.RandomSource;
import net.minecraft.world.DifficultyInstance;
import net.minecraft.world.damagesource.DamageSources;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.entity.EntityTypeTest;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle for the entity predicates advancements match tamed, leashed
 * or bred animals with: real entities made with EntityType.create(LOAD) on a
 * level stand-in, given the case's variant through Entity.setComponent (the
 * variant data component, e.g. minecraft:wolf/variant), then tested with the
 * real EntityPredicate (decoded from the case with the registries' ops), which
 * reads the variant back through Entity.get for its minecraft:components.
 */
public final class VanillaEntityComponentsOracle {
    static Object unsafe;
    static Method allocate, putObject, objectFieldOffset;

    static void put(Object target, Class<?> owner, String field, Object value) throws Exception {
        Field f = owner.getDeclaredField(field);
        long offset = (long) objectFieldOffset.invoke(unsafe, f);
        putObject.invoke(unsafe, target, offset, value);
    }

    static final class Lvl extends ServerLevel {
        Lvl() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        net.minecraft.server.MinecraftServer fake;
        RegistryAccess registries;
        GameRules rules;
        @Override public net.minecraft.server.MinecraftServer getServer() { return fake; }
        int next = 1;
        @Override public RegistryAccess registryAccess() { return registries; }
        @Override public net.minecraft.world.Difficulty getDifficulty() { return net.minecraft.world.Difficulty.NORMAL; }
        @Override public long getGameTime() { return 0L; }
        @Override public int getNextEntityId() { return next++; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
        @Override public GameRules getGameRules() { return rules; }
        @Override public boolean addFreshEntity(Entity entity) { return true; }
        @Override public <T extends Entity> List<T> getEntities(EntityTypeTest<Entity, T> type, AABB bb, Predicate<? super T> selector) { return new ArrayList<>(); }
        net.minecraft.world.attribute.EnvironmentAttributeSystem attributes;
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes() { return attributes; }
        @Override public int getSeaLevel() { return 63; }
        @Override public DifficultyInstance getCurrentDifficultyAt(BlockPos pos) { return new DifficultyInstance(net.minecraft.world.Difficulty.NORMAL, 0L, 0L, 0.0F); }
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        putObject = unsafe.getClass().getMethod("putObject", Object.class, long.class, Object.class);
        objectFieldOffset = unsafe.getClass().getMethod("objectFieldOffset", Field.class);
        RegistryAccess access = ParityData.access;
        Lvl level = (Lvl) allocate.invoke(unsafe, Lvl.class);
        level.registries = access;
        // A server that only answers debug subscribers (PathNavigation asks).
        var server = (net.minecraft.server.MinecraftServer) allocate.invoke(unsafe, net.minecraft.server.dedicated.DedicatedServer.class);
        put(server, net.minecraft.server.MinecraftServer.class, "debugSubscribers", allocate.invoke(unsafe, net.minecraft.util.debug.ServerDebugSubscribers.class));
        level.fake = server;
        put(level, ServerLevel.class, "server", server);
        level.attributes = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().build();
        level.rules = new GameRules(net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags());
        put(level, Level.class, "damageSources", new DamageSources(access));
        put(level, Level.class, "random", RandomSource.create(0L));
        var ops = RegistryOps.create(JsonOps.INSTANCE, ParityData.lookup);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            int n = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                out.println("== " + n++);
                String type = c.get("type").getAsString();
                Entity entity = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(type)).create(level, EntitySpawnReason.LOAD);
                if (c.has("variant")) {
                    // The species' variant component and registry: minecraft:<species>/variant
                    // holds an entry of minecraft:<species>_variant.
                    String species = Identifier.parse(type).getPath();
                    var component = (DataComponentType) BuiltInRegistries.DATA_COMPONENT_TYPE.getValue(Identifier.parse("minecraft:" + species + "/variant"));
                    ResourceKey<Registry<Object>> registry = ResourceKey.createRegistryKey(Identifier.parse("minecraft:" + species + "_variant"));
                    var holder = access.lookupOrThrow(registry).getOrThrow(ResourceKey.create(registry, Identifier.parse(c.get("variant").getAsString())));
                    entity.setComponent(component, holder);
                }
                entity.setPos(0.5, 64.0, 0.5);
                for (var p : c.getAsJsonArray("predicates")) {
                    var predicate = EntityPredicate.CODEC.parse(ops, p).getOrThrow();
                    out.println(predicate.matches(level, new Vec3(2.5, 64.0, 0.5), entity));
                }
            }
        }
        System.exit(0);
    }
}
