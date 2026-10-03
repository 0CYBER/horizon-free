package dev.horizonlab.client;

import com.mojang.brigadier.arguments.IntegerArgumentType;
import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.platform.InputConstants;
import dev.horizonlab.HorizonLab;
import dev.horizonlab.metrics.Recording;
import dev.horizonlab.metrics.Reports;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.command.v2.ClientCommandRegistrationCallback;
import net.fabricmc.fabric.api.client.keymapping.v1.KeyMappingHelper;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Minecraft;
import net.minecraft.client.KeyMapping;
import net.minecraft.network.chat.Component;
import static net.fabricmc.fabric.api.client.command.v2.ClientCommands.argument;
import static net.fabricmc.fabric.api.client.command.v2.ClientCommands.literal;

public final class HorizonClient implements ClientModInitializer {
    private static Recording frames;
    private static long previousStart;
    private static String profile = "custom";

    @Override public void onInitializeClient() {
        KeyMapping benchmarkKey = KeyMappingHelper.registerKeyMapping(new KeyMapping(
            "key.horizonlab.benchmark", InputConstants.KEY_F8, KeyMapping.Category.MISC));
        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            while (benchmarkKey.consumeClick()) {
                if (client.level != null && client.gui.screen() == null) {
                    if (frames == null) start(profile, 120); else finish();
                }
            }
        });
        Path profileFile = FabricLoader.getInstance().getConfigDir().resolve("horizon-profile.txt");
        try {
            if (Files.exists(profileFile)) profile = Files.readString(profileFile).strip();
        } catch (Exception error) { HorizonLab.LOGGER.warn("Profile label could not be read", error); }
        ClientCommandRegistrationCallback.EVENT.register((dispatcher, context) -> dispatcher.register(
            literal("horizon")
                .then(literal("bench")
                    .executes(command -> start("benchmark", 120))
                    .then(argument("label", StringArgumentType.word())
                        .executes(command -> start(StringArgumentType.getString(command, "label"), 120))
                        .then(argument("seconds", IntegerArgumentType.integer(10, 600))
                            .executes(command -> start(StringArgumentType.getString(command, "label"),
                                IntegerArgumentType.getInteger(command, "seconds"))))))
                .then(literal("stop").executes(command -> { finish(); return 1; }))
                .then(literal("status").executes(command -> {
                    message("Profile: " + profile + "; recording: " + (frames != null)); return 1;
                }))));
        HorizonLab.LOGGER.info("Horizon client profile: {}. F8 or /horizon bench <label> [seconds]", profile);
    }

    private static int start(String label, int seconds) {
        if (frames != null) { message("A benchmark is already recording; /horizon stop finishes it."); return 0; }
        Minecraft client = Minecraft.getInstance();
        Path output = client.gameDirectory.toPath().resolve("horizon-benchmarks");
        Map<String, Object> context = new LinkedHashMap<>();
        context.put("minecraft", "26.3");
        context.put("profile", profile);
        context.put("render_distance", client.options.renderDistance().get());
        context.put("simulation_distance", client.options.simulationDistance().get());
        context.put("vsync", client.options.enableVsync().get());
        context.put("fps_limit", client.options.framerateLimit().get());
        context.put("framebuffer_width", client.getWindow().getWidth());
        context.put("framebuffer_height", client.getWindow().getHeight());
        var device = RenderSystem.getDevice().getDeviceInfo();
        context.put("gpu", device.name());
        context.put("graphics_backend", device.backendName());
        context.put("graphics_driver", device.driverInfo());
        context.put("graphics_preset", client.options.graphicsPreset().get().toString());
        context.put("ambient_occlusion", client.options.ambientOcclusion().get());
        context.put("improved_transparency", client.options.improvedTransparency().get());
        context.put("mipmap_levels", client.options.mipmapLevels().get());
        context.put("entity_distance_scaling", client.options.entityDistanceScaling().get());
        context.put("entity_shadows", client.options.entityShadows().get());
        context.put("particles", client.options.particles().get().toString());
        context.put("clouds", client.options.cloudStatus().get().toString());
        context.put("cloud_range", client.options.cloudRange().get());
        context.put("mods", FabricLoader.getInstance().getAllMods().stream()
            .map(mod -> mod.getMetadata().getId() + "@" + mod.getMetadata().getVersion().getFriendlyString())
            .sorted().toList());
        frames = new Recording(label, "frames", output, seconds, context);
        previousStart = 0;
        if (client.getSingleplayerServer() != null) HorizonLab.startTicks(label, seconds, output, profile);
        message("Recording " + seconds + " seconds. Close chat, keep the game focused, and run your route.");
        return 1;
    }

    public static void frameStart() {
        Recording recording = frames;
        if (recording == null) return;
        long now = System.nanoTime();
        if (now >= recording.deadlineNs) { finish(); return; }
        Minecraft client = Minecraft.getInstance();
        if (client.level == null || client.isPaused() || !client.isWindowActive() || client.gui.screen() != null) {
            recording.exclude();
            previousStart = 0;
            return;
        }
        if (previousStart != 0) recording.add(now - previousStart);
        previousStart = now;
    }

    private static void finish() {
        Recording recording = frames;
        if (recording == null) return;
        frames = null;
        previousStart = 0;
        HorizonLab.finishTicks();
        Reports.write(recording.finish()).whenComplete((path, error) -> {
            if (error != null) HorizonLab.LOGGER.error("Frame report failed", error);
            else {
                HorizonLab.LOGGER.info("Horizon frame benchmark saved to {}", path);
                Minecraft.getInstance().execute(() -> message("Benchmark saved: " + path.getFileName()));
            }
        });
    }

    private static void message(String text) {
        Minecraft client = Minecraft.getInstance();
        if (client.gui != null) client.gui.hud.getChat().addClientSystemMessage(Component.literal("[Horizon] " + text));
    }
}
