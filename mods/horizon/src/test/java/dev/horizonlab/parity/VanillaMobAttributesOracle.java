package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.server.level.ServerLevel;

/**
 * Original oracle: instantiates every living entity type on a detached ServerLevel and prints
 * its live attribute map values, standing dimensions (adult and baby), age scale and the
 * base experience reward, one tab separated line per type.
 */
public final class VanillaMobAttributesOracle {
    static final class Level extends ServerLevel {
        Level() { super(null, null, null, null, null, null, false, 0L, java.util.List.of(), false); }
        net.minecraft.server.MinecraftServer fake;
        @Override public net.minecraft.server.MinecraftServer getServer() { return fake; }
        net.minecraft.core.RegistryAccess registries;
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return registries; }
        @Override public net.minecraft.world.Difficulty getDifficulty() { return net.minecraft.world.Difficulty.NORMAL; }
        @Override public long getGameTime() { return 0L; }
        int next = 1;
        @Override public int getNextEntityId() { return next++; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
    }

    static String f(double v) { return Double.toString(v); }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        var uf = u.getDeclaredField("theUnsafe");
        uf.setAccessible(true);
        Object unsafe = uf.get(null);
        Method allocate = u.getMethod("allocateInstance", Class.class);
        Level level = (Level) allocate.invoke(unsafe, Level.class);
        var xp = Mob.class.getDeclaredMethod("getBaseExperienceReward", ServerLevel.class);
        xp.setAccessible(true);
        var ds = net.minecraft.server.dedicated.DedicatedServer.class;
        Object server = allocate.invoke(unsafe, ds);
        Object subs = allocate.invoke(unsafe, net.minecraft.util.debug.ServerDebugSubscribers.class);
        var sf = net.minecraft.server.MinecraftServer.class.getDeclaredField("debugSubscribers");
        sf.setAccessible(true);
        sf.set(server, subs);
        level.fake = (net.minecraft.server.MinecraftServer) server;
        var repo = net.minecraft.server.packs.repository.ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(java.util.List.of("vanilla"));
        var resources = new net.minecraft.server.packs.resources.MultiPackResourceManager(net.minecraft.server.packs.PackType.SERVER_DATA, repo.openAllSelected());
        var layers = net.minecraft.server.RegistryLayer.createRegistryAccess();
        var staticTags = net.minecraft.tags.TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(net.minecraft.server.RegistryLayer.STATIC));
        var context = net.minecraft.tags.TagLoader.buildUpdatedLookups(layers.getAccessForLoading(net.minecraft.server.RegistryLayer.WORLD), staticTags);
        level.registries = net.minecraft.resources.RegistryDataLoader.load(resources, context, net.minecraft.resources.RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (var type : BuiltInRegistries.ENTITY_TYPE) {
                String id = BuiltInRegistries.ENTITY_TYPE.getKey(type).toString();
                Entity e;
                try {
                    e = type.create(level, EntitySpawnReason.LOAD);
                } catch (Throwable t) {
                    out.println(id + "\tFAIL\t" + t);
                    continue;
                }
                if (e == null) { out.println(id + "\tNULL"); continue; }
                var sb = new StringBuilder(id);
                var d = e.getDimensions(Pose.STANDING);
                sb.append("\tdim ").append(f(d.width())).append(' ').append(f(d.height())).append(' ').append(f(d.eyeHeight()));
                if (e instanceof LivingEntity le) {
                    sb.append("\tscale ").append(f(le.getAgeScale()));
                    sb.append("\tattrs");
                    for (var a : BuiltInRegistries.ATTRIBUTE.asHolderIdMap())
                        if (le.getAttributes().hasAttribute(a))
                            sb.append(' ').append(a.unwrapKey().orElseThrow().identifier().getPath()).append('=').append(f(le.getAttributeValue(a)));
                    if (e instanceof AgeableMob am) {
                        try {
                            am.setBaby(true);
                            var b = am.getDimensions(Pose.STANDING);
                            sb.append("\tbaby ").append(f(b.width())).append(' ').append(f(b.height())).append(' ').append(f(b.eyeHeight()));
                        } catch (Throwable t) { sb.append("\tbaby FAIL ").append(t); }
                    } else if (e instanceof net.minecraft.world.entity.monster.zombie.Zombie z) {
                        z.setBaby(true);
                        var b = z.getDimensions(Pose.STANDING);
                        sb.append("\tbaby ").append(f(b.width())).append(' ').append(f(b.height())).append(' ').append(f(b.eyeHeight()));
                    }
                    if (e instanceof Mob m) {
                        try { sb.append("\txp ").append(xp.invoke(m, level)); } catch (Throwable t) { sb.append("\txp ?"); }
                    }
                }
                out.println(sb);
            }
        }
    }
}
