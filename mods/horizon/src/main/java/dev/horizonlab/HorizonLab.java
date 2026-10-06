package dev.horizonlab;

import com.mojang.brigadier.arguments.IntegerArgumentType;
import com.mojang.brigadier.arguments.StringArgumentType;
import dev.horizonlab.metrics.Recording;
import dev.horizonlab.metrics.Reports;
import dev.horizonlab.metrics.ProcessMemoryMonitor;
import dev.horizonlab.metrics.ResidentMemory;
import java.nio.file.Path;
import java.util.Map;
import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.command.v2.CommandRegistrationCallback;
import net.minecraft.commands.Commands;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.permissions.Permissions;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public final class HorizonLab implements ModInitializer {
    public static final Logger LOGGER = LoggerFactory.getLogger("horizonlab");
    private static volatile Recording ticks;

    @Override public void onInitialize() {
        ProcessMemoryMonitor.configure(new ResidentMemory());
        CommandRegistrationCallback.EVENT.register((dispatcher, context, selection) -> dispatcher.register(
            Commands.literal("horizonserver")
                .requires(source -> source.permissions().hasPermission(Permissions.COMMANDS_ADMIN))
                .then(Commands.literal("bench")
                    .then(Commands.argument("label", StringArgumentType.word())
                        .then(Commands.argument("seconds", IntegerArgumentType.integer(10, 600))
                            .executes(command -> {
                                boolean started = startTicks(StringArgumentType.getString(command, "label"),
                                    IntegerArgumentType.getInteger(command, "seconds"), Path.of("horizon-benchmarks"), "server",
                                    command.getSource().getServer().isDedicatedServer() ? "dedicated" : "integrated", Map.of());
                                command.getSource().sendSuccess(() -> Component.literal(started
                                    ? "Horizon: recording tick times; run the intended workload now."
                                    : "Horizon: a tick recording is already active."), false);
                                return started ? 1 : 0;
                            }))))
                .then(Commands.literal("stop").executes(command -> { finishTicks(); return 1; }))));
        LOGGER.info("Horizon ready. Simulation rules and tick rate are unchanged.");
    }

    public static synchronized boolean startTicks(String label, int seconds, Path output, String profile) {
        return startTicks(label, seconds, output, profile, "unknown", Map.of());
    }

    public static synchronized boolean startTicks(String label, int seconds, Path output, String profile,
                                                 String runtimeKind, Map<String, ?> clientContext) {
        if (ticks != null) return false;
        Map<String, Object> context = dev.horizonlab.metrics.SoftwareContext.capture();
        context.put("profile", profile);
        context.put("minecraft", "26.3");
        context.putAll(dev.horizonlab.metrics.ServerContext.capture(runtimeKind, clientContext));
        ticks = new Recording(label, "ticks", output, seconds, context);
        return true;
    }

    public static void tick(long durationNs, MinecraftServer server) {
        Recording recording = ticks;
        if (recording == null) return;
        var tickRate = server.tickRateManager();
        recording.addTick(durationNs, server.getPlayerList().getPlayerCount(), tickRate.tickrate(), tickRate.isFrozen(), tickRate.isSteppingForward(),
            tickRate.isSprinting(), tickRate.runsNormally());
        if (System.nanoTime() >= recording.deadlineNs) finishTicks();
    }

    public static synchronized void finishTicks() {
        Recording recording = ticks;
        if (recording == null) return;
        ticks = null;
        Reports.write(recording.finish()).whenComplete((path, error) -> {
            if (error != null) LOGGER.error("Tick report failed", error);
            else LOGGER.info("Horizon tick benchmark saved to {}", path);
        });
    }
}
