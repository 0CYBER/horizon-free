package dev.horizonlab.metrics;

import com.google.gson.JsonParser;
import com.mojang.blaze3d.systems.RenderSystem;
import java.nio.file.Files;
import java.nio.file.Path;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;

/** Private metadata fixture; ordinary disposable demo, no simulation changes. */
public final class ServerGraphicsIntegration implements ClientModInitializer {
    private boolean opening, finished;
    private long ready, started;

    @Override public void onInitializeClient() {
        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            if (finished || client.gui.overlay() != null) return;
            if (!opening && client.gui.screen() instanceof TitleScreen screen) {
                opening = true;
                client.createWorldOpenFlows().createFreshLevel("Demo_World", MinecraftServer.DEMO_SETTINGS,
                    WorldOptions.DEMO_OPTIONS, WorldPresets::createNormalWorldDimensions, screen);
                return;
            }
            if (client.gui.screen() != null && client.gui.screen().getTitle().getContents() instanceof TranslatableContents title
                && title.getKey().equals("demo.help.title")) { client.gui.setScreen(null); return; }
            if (client.level == null || client.player == null || client.gui.screen() != null || client.isPaused()
                || !client.isWindowActive()) { ready = 0; return; }
            long now = System.nanoTime();
            if (ready == 0) ready = now;
            if (started == 0 && now - ready >= 2_000_000_000L) {
                try {
                    var method = Class.forName("dev.horizonlab.client.HorizonClient").getDeclaredMethod("start", String.class, int.class);
                    method.setAccessible(true);
                    if (!method.invoke(null, "server-graphics-fixture", 10).equals(1)) throw new AssertionError("Recording failed to start");
                    started = now;
                } catch (ReflectiveOperationException error) { throw new AssertionError("Private recorder start failed", error); }
            }
            if (started == 0 || now - started < 14_000_000_000L) return;
            Path output = client.gameDirectory.toPath().resolve("horizon-benchmarks");
            try (var paths = Files.list(output)) {
                var reports = paths.filter(p -> p.toString().endsWith(".json") && p.getFileName().toString().startsWith("server-graphics-fixture-")).toList();
                if (reports.size() != 2) throw new AssertionError("Expected completed frame and tick reports");
                var device = RenderSystem.getDevice().getDeviceInfo();
                var result = new com.google.gson.JsonObject();
                for (Path path : reports) {
                    var report = JsonParser.parseString(Files.readString(path)).getAsJsonObject();
                    if (!report.get("gpu").getAsString().equals(device.name())
                        || !report.get("graphics_backend").getAsString().equals(device.backendName())
                        || !report.get("graphics_driver").getAsString().equals(device.driverInfo()))
                        throw new AssertionError("Report graphics differ from actual device");
                    if (report.get("kind").getAsString().equals("ticks")) {
                        if (!report.get("server_runtime_kind").getAsString().equals("integrated")
                            || !report.get("client_graphics_capture_status").getAsString().equals("captured_at_recording_start"))
                            throw new AssertionError("Integrated graphics status missing");
                        for (String key : new String[]{"server_runtime_kind", "client_graphics_capture_status", "gpu", "graphics_backend", "graphics_driver"}) result.add(key, report.get(key));
                    }
                }
                finished = true;
                System.out.println("HORIZON_SERVER_GRAPHICS_RESULT " + result);
                client.stop();
            } catch (Exception error) { throw new AssertionError("Private server graphics fixture failed", error); }
        });
    }
}
