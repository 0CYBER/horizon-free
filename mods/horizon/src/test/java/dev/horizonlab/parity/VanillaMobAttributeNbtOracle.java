package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.item.ItemStack;

/**
 * Original oracle: for each case line (entity id, saved NBT, equipment), loads the entity on a
 * detached ServerLevel, equips it and runs LivingEntity.collectEquipmentChanges, then prints the
 * attribute values and bases asked for and the saved `attributes` list, as exact double bits.
 */
public final class VanillaMobAttributeNbtOracle {
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

    static String bits(double v) { return Long.toHexString(Double.doubleToRawLongBits(v)); }

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
        var world = net.minecraft.resources.RegistryDataLoader.load(resources, context, net.minecraft.resources.RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        level.registries = world;
        // Items read their components (attribute modifiers) only once the initializers ran.
        var loaded = net.minecraft.server.ReloadableServerRegistries.reload(layers.replaceFrom(net.minecraft.server.RegistryLayer.WORLD, world), staticTags, resources, Runnable::run).join();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(loaded.lookupWithUpdatedTags());
        staticTags.forEach(net.minecraft.core.Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        var attrs = BuiltInRegistries.ATTRIBUTE;
        var items = BuiltInRegistries.ITEM;
        var collect = LivingEntity.class.getDeclaredMethod("collectEquipmentChanges", java.util.Map.class);
        collect.setAccessible(true);
        var lastField = LivingEntity.class.getDeclaredField("lastEquipmentItems");
        lastField.setAccessible(true);
        var gson = new com.google.gson.Gson();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                if (line.isBlank()) continue;
                var c = gson.fromJson(line, com.google.gson.JsonObject.class);
                var sb = new StringBuilder(c.get("name").getAsString());
                try {
                    var type = BuiltInRegistries.ENTITY_TYPE.get(net.minecraft.resources.Identifier.parse(c.get("entity").getAsString())).get().value();
                    Entity e = type.create(level, EntitySpawnReason.LOAD);
                    var nbt = net.minecraft.nbt.TagParser.parseCompoundFully(c.get("snbt").getAsString());
                    e.load(net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess(), nbt));
                    var living = (LivingEntity) e;
                    for (var eq : c.getAsJsonObject("equip").entrySet()) {
                        living.setItemSlot(EquipmentSlot.byName(eq.getKey()), new ItemStack(items.get(net.minecraft.resources.Identifier.parse(eq.getValue().getAsString())).get()));
                    }
                    @SuppressWarnings("unchecked")
                    var last = (java.util.Map<EquipmentSlot, ItemStack>) lastField.get(living);
                    var changed = (java.util.Map<EquipmentSlot, ItemStack>) collect.invoke(living, last);
                    // Remove then re-add on a second pass, as the next tick would after a swap.
                    for (var eq : c.getAsJsonObject("then").entrySet()) {
                        living.setItemSlot(EquipmentSlot.byName(eq.getKey()), eq.getValue().getAsString().isEmpty() ? ItemStack.EMPTY : new ItemStack(items.get(net.minecraft.resources.Identifier.parse(eq.getValue().getAsString())).get()));
                    }
                    if (changed != null) for (var en : changed.entrySet()) last.put(en.getKey(), en.getValue().copy());
                    if (!c.getAsJsonObject("then").isEmpty()) collect.invoke(living, last);
                    var map = living.getAttributes();
                    for (var name : c.getAsJsonArray("read")) {
                        var holder = attrs.get(net.minecraft.resources.Identifier.parse(name.getAsString())).get();
                        if (!map.hasAttribute(holder)) { sb.append("\tread ").append(name.getAsString()).append(" none"); continue; }
                        sb.append("\tread ").append(name.getAsString()).append(' ').append(bits(map.getValue(holder))).append(' ').append(bits(map.getBaseValue(holder)));
                    }
                    var output = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess());
                    e.saveWithoutId(output);
                    var list = output.buildResult().getListOrEmpty("attributes");
                    var rows = new java.util.TreeMap<String, String>();
                    for (int i = 0; i < list.size(); i++) {
                        var a = list.getCompoundOrEmpty(i);
                        var row = new StringBuilder("base ").append(bits(a.getDoubleOr("base", 0.0)));
                        var mods = a.getListOrEmpty("modifiers");
                        for (int j = 0; j < mods.size(); j++) {
                            var m = mods.getCompoundOrEmpty(j);
                            row.append(" mod ").append(m.getStringOr("id", "?")).append(' ').append(bits(m.getDoubleOr("amount", 0.0))).append(' ').append(m.getStringOr("operation", "?"));
                        }
                        rows.put(a.getStringOr("id", "?"), row.toString());
                    }
                    rows.forEach((id, row) -> sb.append("\tsaved ").append(id).append(' ').append(row));
                } catch (Throwable t) {
                    var cause = t instanceof java.lang.reflect.InvocationTargetException ? t.getCause() : t;
                    sb.append("\tFAIL ").append(cause);
                    cause.printStackTrace();
                }
                out.println(sb);
            }
        }
    }
}
