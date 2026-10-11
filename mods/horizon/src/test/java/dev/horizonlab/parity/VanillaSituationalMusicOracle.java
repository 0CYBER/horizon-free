package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.gui.components.BossHealthOverlay;
import net.minecraft.client.gui.components.LerpingBossEvent;
import net.minecraft.client.gui.screens.WinScreen;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.core.*;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.sounds.Music;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.BossEvent;
import net.minecraft.world.attribute.*;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.phys.Vec3;

/**
 * Calls the real Minecraft.getSituationalMusic and getMusicVolume on shells
 * holding the open screen (none or the credits' WinScreen), the boss overlay's
 * events, the player (its level's dimension, abilities and underwater state)
 * and a real camera EnvironmentAttributeProbe over the dimension and biome's
 * attribute layers from the vanilla data pack. Lines: "screen player
 * dimension biome boss instabuild mayfly underwater".
 */
public final class VanillaSituationalMusicOracle {
    static String music(Music m) {
        return m == null ? "-" : m.sound().value().location() + "," + m.minDelay() + "," + m.maxDelay() + "," + m.replaceCurrentMusic();
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void addBiome(EnvironmentAttributeSystem.Builder b, EnvironmentAttribute attr, Biome biome) {
        b.addPositionalLayer(attr, (base, pos, w) -> biome.getAttributes().applyModifier(attr, base));
    }

    static LerpingBossEvent bar(boolean darken, boolean music) {
        return new LerpingBossEvent(UUID.randomUUID(), Component.empty(), 1.0F, BossEvent.BossBarColor.PINK, BossEvent.BossBarOverlay.PROGRESS,
            darken, music, false);
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

        Minecraft mc = VanillaSoundRig.make(Minecraft.class);
        Gui gui = VanillaSoundRig.make(Gui.class);
        Hud hud = VanillaSoundRig.make(Hud.class);
        BossHealthOverlay overlay = VanillaSoundRig.make(BossHealthOverlay.class);
        Map<UUID, LerpingBossEvent> events = new LinkedHashMap<>();
        VanillaSoundRig.set(overlay, BossHealthOverlay.class, "events", events);
        VanillaSoundRig.set(hud, Hud.class, "bossOverlay", overlay);
        VanillaSoundRig.set(gui, Gui.class, "hud", hud);
        VanillaSoundRig.set(mc, Minecraft.class, "gui", gui);
        GameRenderer renderer = VanillaSoundRig.make(GameRenderer.class);
        Camera camera = VanillaSoundRig.make(Camera.class);
        VanillaSoundRig.set(renderer, GameRenderer.class, "mainCamera", camera);
        VanillaSoundRig.set(mc, Minecraft.class, "gameRenderer", renderer);
        WinScreen credits = VanillaSoundRig.make(WinScreen.class);
        try (var w = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
                String[] f = line.split(" ");
                VanillaSoundRig.set(gui, Gui.class, "screen", f[0].equals("win") ? credits : null);
                events.clear();
                if (f[4].contains("dark")) events.put(UUID.randomUUID(), bar(true, false));
                if (f[4].contains("music")) events.put(UUID.randomUUID(), bar(false, true));
                var probe = new EnvironmentAttributeProbe();
                VanillaSoundRig.set(camera, Camera.class, "attributeProbe", probe);
                if (f[1].equals("1")) {
                    DimensionType dim = dimensions.getOrThrow(ResourceKey.create(Registries.DIMENSION_TYPE, Identifier.parse(f[2]))).value();
                    Biome biome = biomes.getOrThrow(ResourceKey.create(Registries.BIOME, Identifier.parse(f[3]))).value();
                    var b = EnvironmentAttributeSystem.builder();
                    b.addConstantLayer(dim.attributes());
                    for (EnvironmentAttribute<?> attr : biome.getAttributes().keySet()) addBiome(b, attr, biome);
                    ClientLevel level = VanillaSoundRig.make(ClientLevel.class);
                    VanillaSoundRig.set(level, ClientLevel.class, "environmentAttributes", b.build());
                    VanillaSoundRig.set(level, Level.class, "dimension", f[2].equals("minecraft:the_end") ? Level.END
                        : f[2].equals("minecraft:the_nether") ? Level.NETHER : Level.OVERWORLD);
                    VanillaSoundRig.set(probe, EnvironmentAttributeProbe.class, "level", level);
                    VanillaSoundRig.set(probe, EnvironmentAttributeProbe.class, "position", new Vec3(0.5, 64.0, 0.5));
                    LocalPlayer player = VanillaSoundRig.make(LocalPlayer.class);
                    VanillaSoundRig.set(player, Entity.class, "level", level);
                    var abilities = new Abilities();
                    abilities.instabuild = f[5].equals("1");
                    abilities.mayfly = f[6].equals("1");
                    VanillaSoundRig.set(player, Player.class, "abilities", abilities);
                    VanillaSoundRig.set(player, Player.class, "wasUnderwater", f[7].equals("1"));
                    mc.player = player;
                } else {
                    mc.player = null;
                }
                w.printf("%s music=%s volume=%08x%n", line, music(mc.getSituationalMusic()), Float.floatToRawIntBits(mc.getMusicVolume()));
            }
        }
        System.exit(0);
    }
}
