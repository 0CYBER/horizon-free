package dev.horizonlab.metrics;

import com.google.gson.JsonObject;
import com.mojang.blaze3d.systems.RenderSystem;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.HashSet;
import java.util.HexFormat;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.minecraft.client.Screenshot;
import net.minecraft.client.gui.screens.TitleScreen;

/** Private startup/readback probe. Never packaged in a playable release. */
public final class ClientGraphicsIntegration implements ClientModInitializer {
    private long titleStarted;
    private int titleTicks;
    private boolean requested;

    @Override public void onInitializeClient() {
        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            if (requested || !(client.gui.screen() instanceof TitleScreen)) return;
            if (titleStarted == 0) titleStarted = System.nanoTime();
            titleTicks++;
            if (titleTicks < 40 || System.nanoTime() - titleStarted < 2_000_000_000L) return;
            requested = true;
            Screenshot.takeScreenshot(client.gameRenderer.mainRenderTarget(), image -> {
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
                    if (samples < 256 || colors.size() < 8) throw new AssertionError("Uniform title readback");
                    var screenshot = client.gameDirectory.toPath().resolve("client-graphics-private.png");
                    image.writeToFile(screenshot);
                    var device = RenderSystem.getDevice().getDeviceInfo();
                    var result = new JsonObject();
                    result.addProperty("title_screen_observed", true);
                    result.addProperty("title_client_ticks", titleTicks);
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
