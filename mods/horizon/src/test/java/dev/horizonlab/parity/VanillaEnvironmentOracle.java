package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.attribute.*;
import net.minecraft.world.clock.*;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.phys.Vec3;
import org.joml.Vector3fc;
import org.joml.Vector4fc;

/**
 * Original oracle: builds the vanilla EnvironmentAttributeSystem (dimension
 * layer, biome layer for one fixed biome, dimension timelines, weather layers)
 * from the vanilla data pack and prints every visual/gameplay attribute for
 * fixture lines "dimension ticks rain thunder biome".
 */
public final class VanillaEnvironmentOracle {
    private static String fmt(Object v) {
        if (v instanceof Vector3fc c) return String.format("[%s,%s,%s]", c.x(), c.y(), c.z());
        if (v instanceof Vector4fc c) return String.format("[%s,%s,%s,%s]", c.x(), c.y(), c.z(), c.w());
        if (v instanceof Float f) return f.toString();
        if (v instanceof Boolean b) return b.toString();
        if (v instanceof Enum<?> e) return "\"" + e.name() + "\"";
        return null;
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void addBiome(EnvironmentAttributeSystem.Builder b, EnvironmentAttribute attr, Biome biome) {
        b.addPositionalLayer(attr, (base, pos, w) -> biome.getAttributes().applyModifier(attr, base));
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Path in = Path.of(args[0]), out = Path.of(args[1]);
        var repo = ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        var resources = new MultiPackResourceManager(PackType.SERVER_DATA, repo.openAllSelected());
        var layers = RegistryLayer.createRegistryAccess();
        var staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
        RegistryAccess.Frozen world = RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        staticTags.forEach(Registry.PendingTags::apply);
        var dimensions = world.lookupOrThrow(Registries.DIMENSION_TYPE);
        var biomes = world.lookupOrThrow(Registries.BIOME);
        try (var w = new PrintWriter(Files.newBufferedWriter(out, StandardCharsets.UTF_8))) {
            for (String line : Files.readAllLines(in, StandardCharsets.UTF_8)) {
                String[] f = line.split(" ");
                DimensionType dim = dimensions.getOrThrow(ResourceKey.create(Registries.DIMENSION_TYPE, Identifier.withDefaultNamespace(f[0]))).value();
                long ticks = Long.parseLong(f[1]);
                float rain = Float.parseFloat(f[2]), thunder = Float.parseFloat(f[3]);
                Biome biome = biomes.getOrThrow(ResourceKey.create(Registries.BIOME, Identifier.withDefaultNamespace(f[4]))).value();
                ClockManager clocks = h -> new ClockInstance() {
                    public long totalTicks() { return ticks; }
                    public float partialTick() { return 0.0F; }
                    public float rate() { return 1.0F; }
                    public boolean isPaused() { return false; }
                };
                var b = EnvironmentAttributeSystem.builder();
                b.addConstantLayer(dim.attributes());
                for (EnvironmentAttribute<?> attr : biome.getAttributes().keySet()) addBiome(b, attr, biome);
                dim.timelines().forEach(t -> b.addTimelineLayer(t, clocks));
                if (dim.hasSkyLight() && !dim.hasCeiling()) {
                    WeatherAttributes.addBuiltinLayers(b, new WeatherAttributes.WeatherAccess() {
                        public float rainLevel() { return rain; }
                        public float thunderLevel() { return thunder; }
                    });
                }
                var system = b.build();
                StringBuilder sb = new StringBuilder("{\"case\":\"" + line + "\"");
                for (var entry : BuiltInRegistries.ENVIRONMENT_ATTRIBUTE.entrySet()) {
                    EnvironmentAttribute<?> attr = entry.getValue();
                    String s = fmt(system.getValue(attr, Vec3.ZERO));
                    if (s != null) sb.append(",\"").append(entry.getKey().identifier()).append("\":").append(s);
                }
                w.println(sb.append("}"));
            }
        }
    }
}
