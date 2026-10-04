package dev.horizonlab.metrics;

import com.google.gson.JsonObject;
import com.mojang.blaze3d.systems.RenderSystem;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.HashSet;
import java.util.HexFormat;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Screenshot;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;

/** Private startup/readback probe. Never packaged in a playable release. */
public final class ClientGraphicsIntegration implements ClientModInitializer {
    private long titleStarted;
    private int titleTicks;
    private boolean requested;
    private boolean opening;
    private final boolean worldMode = Files.exists(FabricLoader.getInstance().getConfigDir().resolve("client-graphics-world.flag"));

    @Override public void onInitializeClient() {
        System.out.println("HORIZON_CLIENT_GRAPHICS_INITIALIZER_READY");
        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            if (requested) return;
            if (worldMode) {
                if (!opening && client.gui.screen() instanceof TitleScreen screen) {
                    opening = true;
                    if (Files.isRegularFile(client.gameDirectory.toPath().resolve("saves/Demo_World/level.dat"))) {
                        client.createWorldOpenFlows().openWorld("Demo_World", () -> {
                            throw new AssertionError("Disposable graphics world did not open");
                        });
                    } else {
                        client.createWorldOpenFlows().createFreshLevel("Demo_World", MinecraftServer.DEMO_SETTINGS,
                            WorldOptions.DEMO_OPTIONS, WorldPresets::createNormalWorldDimensions, screen);
                    }
                    return;
                }
                if (client.gui.screen() != null && client.gui.screen().getTitle().getContents() instanceof TranslatableContents title
                    && title.getKey().equals("demo.help.title")) {
                    client.gui.setScreen(null);
                    return;
                }
                if (client.level == null || client.player == null || client.gui.screen() != null
                    || client.isPaused() || !client.isWindowActive()) return;
            } else if (!(client.gui.screen() instanceof TitleScreen)) return;
            if (titleStarted == 0) titleStarted = System.nanoTime();
            titleTicks++;
            if (titleTicks < 40 || System.nanoTime() - titleStarted < 2_000_000_000L) return;
            requested = true;
            System.out.println("HORIZON_CLIENT_GRAPHICS_READBACK_REQUESTED");
            Screenshot.takeScreenshot(client.gameRenderer.mainRenderTarget(), image -> {
                System.out.println("HORIZON_CLIENT_GRAPHICS_READBACK_RECEIVED");
                try (image) {
                    int width = image.getWidth(), height = image.getHeight();
                    if (width < 64 || height < 64) throw new AssertionError("Incomplete title framebuffer");
                    var colors = new HashSet<Integer>();
                    int samples = 0;
                    for (int y = 0; y < height; y += Math.max(1, height / 64)) {
                        for (int x = 0; x < width; x += Math.max(1, width / 64)) {
                            colors.add(image.getPixel(x, y) & 0xFFFFFF);
                            samples++;
                        }
                    }
                    var inspection = new JsonObject();
                    inspection.addProperty("framebuffer_width", width);
                    inspection.addProperty("framebuffer_height", height);
                    inspection.addProperty("sampled_pixels", samples);
                    inspection.addProperty("distinct_sampled_rgb", colors.size());
                    System.out.println("HORIZON_CLIENT_GRAPHICS_READBACK_INSPECTED " + inspection);
                    if (samples < 256 || colors.size() < 8) throw new AssertionError("Uniform title readback");
                    var screenshot = client.gameDirectory.toPath().resolve("client-graphics-private.png");
                    image.writeToFile(screenshot);
                    var device = RenderSystem.getDevice().getDeviceInfo();
                    var result = new JsonObject();
                    result.addProperty("observed_mode", worldMode ? "world" : "title");
                    result.addProperty("title_screen_observed", !worldMode);
                    result.addProperty("world_loaded", worldMode && client.level != null);
                    result.addProperty("ready_client_ticks", titleTicks);
                    result.addProperty("window_active", client.isWindowActive());
                    result.addProperty("framebuffer_width", width);
                    result.addProperty("framebuffer_height", height);
                    result.addProperty("sampled_pixels", samples);
                    result.addProperty("distinct_sampled_rgb", colors.size());
                    result.addProperty("gpu", device.name());
                    result.addProperty("graphics_backend", device.backendName());
                    result.addProperty("graphics_driver", device.driverInfo());
                    result.addProperty("private_png_sha256", HexFormat.of().formatHex(
                        MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(screenshot))));
                    System.out.println("HORIZON_CLIENT_GRAPHICS_RESULT " + result);
                    client.execute(client::stop);
                } catch (Exception error) {
                    throw new AssertionError("Private client graphics readback failed", error);
                }
            });
        });
    }
}
