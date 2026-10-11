package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.sounds.Music;
import net.minecraft.sounds.Musics;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.attribute.*;
import net.minecraft.world.clock.*;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.phys.Vec3;

/**
 * The audio environment attributes as the vanilla EnvironmentAttributeSystem
 * evaluates them from the vanilla data pack (dimension layer, the biome's
 * positional layer, the dimension timelines and weather layers):
 * BACKGROUND_MUSIC with BackgroundMusic.select for every creative/underwater
 * combination, MUSIC_VOLUME and AMBIENT_SOUNDS, for lines "dimension biome
 * ticks"; plus the Musics constants.
 */
public final class VanillaAudioAttributesOracle {
    static String music(Music m) {
        return m == null ? "-" : m.sound().value().location() + "," + m.minDelay() + "," + m.maxDelay() + "," + m.replaceCurrentMusic();
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void addBiome(EnvironmentAttributeSystem.Builder b, EnvironmentAttribute attr, Biome biome) {
        b.addPositionalLayer(attr, (base, pos, w) -> biome.getAttributes().applyModifier(attr, base));
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
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
        try (var w = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            w.println("MENU " + music(Musics.MENU));
            w.println("CREDITS " + music(Musics.CREDITS));
            w.println("END_BOSS " + music(Musics.END_BOSS));
            w.println("CREATIVE " + music(Musics.CREATIVE));
            w.println("END " + music(Musics.END));
            w.println("UNDER_WATER " + music(Musics.UNDER_WATER));
            w.println("GAME " + music(Musics.GAME));
            for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
                String[] f = line.split(" ");
                DimensionType dim = dimensions.getOrThrow(ResourceKey.create(Registries.DIMENSION_TYPE, Identifier.parse(f[0]))).value();
                Biome biome = biomes.getOrThrow(ResourceKey.create(Registries.BIOME, Identifier.parse(f[1]))).value();
                long ticks = Long.parseLong(f[2]);
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
                        public float rainLevel() { return 0.0F; }
                        public float thunderLevel() { return 0.0F; }
                    });
                }
                var system = b.build();
                BackgroundMusic bg = system.getValue(EnvironmentAttributes.BACKGROUND_MUSIC, Vec3.ZERO);
                StringBuilder sb = new StringBuilder(line);
                sb.append(" default=").append(music(bg.defaultMusic().orElse(null)))
                    .append(" creative=").append(music(bg.creativeMusic().orElse(null)))
                    .append(" underwater=").append(music(bg.underwaterMusic().orElse(null)));
                for (boolean creative : new boolean[]{false, true}) {
                    for (boolean underwater : new boolean[]{false, true}) {
                        sb.append(" select=").append(music(bg.select(creative, underwater).orElse(null)));
                    }
                }
                sb.append(String.format(" volume=%08x", Float.floatToRawIntBits(system.getValue(EnvironmentAttributes.MUSIC_VOLUME, Vec3.ZERO))));
                AmbientSounds ambient = system.getValue(EnvironmentAttributes.AMBIENT_SOUNDS, Vec3.ZERO);
                sb.append(" loop=").append(ambient.loop().map(s -> s.value().location().toString()).orElse("-"));
                sb.append(" mood=").append(ambient.mood().map(m -> m.soundEvent().value().location() + "," + m.tickDelay() + ","
                    + m.blockSearchExtent() + "," + String.format("%016x", Double.doubleToRawLongBits(m.soundPositionOffset()))).orElse("-"));
                for (AmbientAdditionsSettings a : ambient.additions()) {
                    sb.append(" addition=").append(a.soundEvent().value().location()).append(String.format(",%016x", Double.doubleToRawLongBits(a.tickChance())));
                }
                w.println(sb);
            }
        }
        System.exit(0);
    }
}
