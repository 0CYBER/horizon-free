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
import net.minecraft.client.Options;
import net.minecraft.client.KeyMapping;
import net.minecraft.network.chat.Component;
import static net.fabricmc.fabric.api.client.command.v2.ClientCommands.argument;
import static net.fabricmc.fabric.api.client.command.v2.ClientCommands.literal;

public final class HorizonClient implements ClientModInitializer {
    private static Recording frames;
    private static long previousStart;
    private static String profile = "custom";
    private static DistantTerrainBudget terrainBudget;
    private static long previousBudgetFrame;
    private static int targetFps = 60;

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
        Path budgetFile = FabricLoader.getInstance().getConfigDir().resolve("horizon-adaptive.properties");
        try {
            if (Files.exists(budgetFile)) {
                var properties = new java.util.Properties();
                try (var reader = Files.newBufferedReader(budgetFile)) { properties.load(reader); }
                targetFps = Math.clamp(Integer.parseInt(properties.getProperty("targetFps", "60")), 10, 240);
                if (Boolean.parseBoolean(properties.getProperty("enabled", "false"))) enableBudget();
            }
        } catch (Exception error) { HorizonLab.LOGGER.warn("Adaptive budget disabled: invalid configuration", error); }
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
                .then(literal("adaptive")
                    .then(literal("on").executes(command -> {
                        if (frames != null) { message("Finish the recording before changing the budget"); return 0; }
                        message(enableBudget() ? "Adaptive terrain budget enabled" : "Distant Horizons is required");
                        return 1;
                    }))
                    .then(literal("off").executes(command -> {
                        if (frames != null) { message("Finish the recording before changing the budget"); return 0; }
                        if (terrainBudget != null) terrainBudget.suspend();
                        terrainBudget = null;
                        previousBudgetFrame = 0;
                        message("Adaptive terrain budget disabled"); return 1;
                    })))
                .then(literal("status").executes(command -> {
                    message("Profile: " + profile + "; recording: " + (frames != null)
                        + "; adaptive: " + (terrainBudget == null ? "off" : terrainBudget.status())); return 1;
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
        context.put("adaptive_budget_enabled", terrainBudget != null);
        context.put("precise_worker_pacing", dev.horizonlab.mixin.HorizonMixinPlugin.workerPacingEnabled());
        context.put("debounce_queue_wait", dev.horizonlab.mixin.HorizonMixinPlugin.debounceWaitEnabled());
        context.put("wakeable_idle_wait", dev.horizonlab.mixin.HorizonMixinPlugin.debounceWaitEnabled());
        context.put("dispatch_retry_backoff", dev.horizonlab.mixin.HorizonMixinPlugin.dispatchRetryEnabled());
        context.put("tick_membership_dedup", dev.horizonlab.mixin.HorizonMixinPlugin.tickDedupEnabled());
        context.put("terrain_quad_reuse", dev.horizonlab.mixin.HorizonMixinPlugin.quadReuseEnabled());
        context.put("terrain_position_arithmetic", dev.horizonlab.mixin.HorizonMixinPlugin.terrainPositionEnabled());
        context.put("culling_ray_scratch", dev.horizonlab.mixin.HorizonMixinPlugin.cullingRayScratchEnabled());
        context.put("empty_culling_whitelist", dev.horizonlab.mixin.HorizonMixinPlugin.cullingEmptyEnabled());
        context.put("interruptible_terrain_read", dev.horizonlab.mixin.HorizonMixinPlugin.terrainReadWaitEnabled());
        context.put("adaptive_target_fps", targetFps);
        context.putAll(visualSettings(client));
        var device = RenderSystem.getDevice().getDeviceInfo();
        context.put("gpu", device.name());
        context.put("graphics_backend", device.backendName());
        context.put("graphics_driver", device.driverInfo());
        context.putAll(dev.horizonlab.metrics.SoftwareContext.capture());
        frames = new Recording(label, "frames", output, seconds, context);
        previousStart = 0;
        if (client.getSingleplayerServer() != null) HorizonLab.startTicks(label, seconds, output, profile, "integrated", context);
        message("Recording " + seconds + " seconds. Close chat, keep the game focused, and run your route.");
        return 1;
    }

    private static Map<String, Object> visualSettings(Minecraft client) {
        Map<String, Object> context = new LinkedHashMap<>();
        if (client.level != null) context.put("world_difficulty", client.level.getDifficulty().name());
        context.put("render_distance", client.options.renderDistance().get());
        context.put("simulation_distance", client.options.simulationDistance().get());
        context.put("vsync", client.options.enableVsync().get());
        context.put("fps_limit", client.options.framerateLimit().get());
        context.put("fps_limit_unlimited", client.options.framerateLimit().get() >= Options.UNLIMITED_FRAMERATE_CUTOFF);
        context.put("framebuffer_width", client.getWindow().getWidth());
        context.put("framebuffer_height", client.getWindow().getHeight());
        context.put("graphics_preset", client.options.graphicsPreset().get().toString());
        context.put("ambient_occlusion", client.options.ambientOcclusion().get());
        context.put("improved_transparency", client.options.improvedTransparency().get());
        context.put("mipmap_levels", client.options.mipmapLevels().get());
        context.put("entity_distance_scaling", client.options.entityDistanceScaling().get());
        context.put("entity_shadows", client.options.entityShadows().get());
        context.put("particles", client.options.particles().get().toString());
        context.put("clouds", client.options.cloudStatus().get().toString());
        context.put("cloud_range", client.options.cloudRange().get());
        context.put("fov", client.options.fov().get());
        var loader = FabricLoader.getInstance();
        if (loader.isModLoaded("distanthorizons")) {
            try { context.putAll(DistantTerrainSettings.read()); }
            catch (RuntimeException | LinkageError unavailable) { context.put("dh_capture_status", "unavailable"); }
        } else context.put("dh_capture_status", "not_installed");
        context.putAll(ShaderSettings.read(loader.isModLoaded("iris")));
        return context;
    }

    public static void frameStart() {
        Recording recording = frames;
        if (recording == null && terrainBudget == null) return;
        long now = System.nanoTime();
        if (recording != null && now >= recording.deadlineNs) { finish(); recording = null; }
        Minecraft client = Minecraft.getInstance();
        if (client.level == null || client.isPaused() || !client.isWindowActive() || client.gui.screen() != null) {
            if (recording != null) recording.exclude();
            if (terrainBudget != null && previousBudgetFrame != 0) terrainBudget.suspend();
            previousBudgetFrame = 0;
            previousStart = 0;
            return;
        }
        if (terrainBudget != null) {
            int cap = client.options.framerateLimit().get();
            double targetMs = 1000.0 / Math.min(targetFps, Math.max(1, cap));
            terrainBudget.observe(now, previousBudgetFrame == 0 ? 0 : now - previousBudgetFrame, targetMs);
            previousBudgetFrame = now;
        }
        if (recording != null) {
            if (previousStart != 0) recording.add(now - previousStart);
            previousStart = now;
        }
    }

    private static boolean enableBudget() {
        if (!FabricLoader.getInstance().isModLoaded("distanthorizons")) return false;
        if (terrainBudget == null) terrainBudget = new DistantTerrainBudget();
        return true;
    }

    private static void finish() {
        Recording recording = frames;
        if (recording == null) return;
        frames = null;
        previousStart = 0;
        HorizonLab.finishTicks();
        Map<String, Object> end = visualSettings(Minecraft.getInstance());
        java.util.List<String> changed = new java.util.ArrayList<>();
        for (var setting : end.entrySet()) {
            // Adaptive duty cycle is expected to change; visual quality changes are separate.
            if (!setting.getKey().equals("dh_runtime_ratio")
                    && !java.util.Objects.equals(recording.context.get(setting.getKey()), setting.getValue()))
                changed.add(setting.getKey());
        }
        for (String setting : recording.context.keySet()) {
            if ((setting.startsWith("dh_") || setting.startsWith("iris_")) && !end.containsKey(setting))
                changed.add(setting);
        }
        recording.context.put("visual_settings_end", end);
        recording.context.put("visual_settings_changed", changed);
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
