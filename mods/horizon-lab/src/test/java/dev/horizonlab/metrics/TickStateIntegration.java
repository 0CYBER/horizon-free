package dev.horizonlab.metrics;

import com.google.gson.JsonParser;
import dev.horizonlab.HorizonLab;
import java.nio.file.Files;
import java.nio.file.Path;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;

/** Private fixture. Run only on a disposable demo world; never distribute this mod. */
public final class TickStateIntegration implements ClientModInitializer {
    private int phase;
    private long started;

    @Override public void onInitializeClient() {
        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            var server = client.getSingleplayerServer();
            if (server == null || client.level == null || client.isPaused() || client.gui.screen() != null) return;
            Path output = client.gameDirectory.toPath().resolve("horizon-benchmarks");
            if (phase == 0) {
                phase++;
                started = System.nanoTime();
                server.execute(() -> {
                    var manager = server.tickRateManager();
                    if (manager.tickrate() != 20 || manager.isFrozen() || manager.isSprinting())
                        throw new AssertionError("Fixture requires normal initial tick-manager state");
                    if (!HorizonLab.startTicks("tick-state-fixture", 10, output, "private-fixture"))
                        throw new AssertionError("Fixture recording was not started");
                });
            }
            double elapsed = (System.nanoTime() - started) / 1e9;
            if (phase == 1 && elapsed >= 2) {
                phase++;
                server.execute(() -> { server.tickRateManager().setTickRate(40); server.tickRateManager().setFrozen(true); });
            } else if (phase == 2 && elapsed >= 4) {
                phase++;
                server.execute(() -> {
                    if (!server.tickRateManager().stepGameIfPaused(10)) throw new AssertionError("Fixture step failed");
                });
            } else if (phase == 3 && elapsed >= 6) {
                phase++;
                server.execute(() -> { server.tickRateManager().setFrozen(false); server.tickRateManager().setTickRate(20); });
            } else if (phase == 4 && elapsed >= 8) {
                phase++;
                server.execute(() -> server.tickRateManager().requestGameToSprint(5));
            } else if (phase == 5 && elapsed >= 12) {
                try (var paths = Files.list(output)) {
                    var reports = paths.filter(p -> p.getFileName().toString().startsWith("tick-state-fixture-ticks-")
                        && p.toString().endsWith(".json")).toList();
                    if (reports.size() != 1) throw new AssertionError("Expected one completed fixture report");
                    var report = JsonParser.parseString(Files.readString(reports.getFirst())).getAsJsonObject();
                    if (!report.get("tick_state_first").equals(report.get("tick_state_last")))
                        throw new AssertionError("Fixture did not restore its boundary state");
                    if (report.get("tick_state_samples").getAsLong() != report.getAsJsonObject("stats").get("count").getAsLong())
                        throw new AssertionError("Applied tick-state and duration counts differ");
                    for (String field : new String[]{"frozen", "stepping", "sprinting", "suppressed", "nonstandard_rate"})
                        if (report.get("tick_state_" + field + "_samples").getAsLong() < 1)
                            throw new AssertionError("Applied fixture did not observe " + field);
                    var result = new com.google.gson.JsonObject();
                    for (var entry : report.entrySet())
                        if (entry.getKey().startsWith("tick_state_") || entry.getKey().equals("observed_ticks_per_second"))
                            result.add(entry.getKey(), entry.getValue());
                    System.out.println("HORIZON_TICK_STATE_RESULT " + result);
                    phase++;
                    client.stop();
                } catch (Exception error) { throw new AssertionError("Private applied tick-state fixture failed", error); }
            }
        });
    }
}
