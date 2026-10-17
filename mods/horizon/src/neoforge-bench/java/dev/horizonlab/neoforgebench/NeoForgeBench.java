package dev.horizonlab.neoforgebench;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.blaze3d.platform.GlUtil;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.Difficulty;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.phys.Vec3;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.client.event.RenderFrameEvent;
import net.neoforged.neoforge.common.NeoForge;

/**
 * Dev-only benchmark driver for a NeoForge 1.21.1 client (tools/perf_bench.py
 * --game neoforge): the steps of mods/horizon/src/bench's VanillaBench, on
 * NeoForge's events. From the title screen it creates a fresh creative world
 * with the script's seed, waits for the spawn area, then per scene teleports,
 * waits for the chunks, settles and flies a straight line at constant speed,
 * recording every frame time. Events go to horizon.bench.out as JSON lines;
 * the game quits after the last scene. Never shipped.
 */
@Mod(value = "horizon_bench", dist = Dist.CLIENT)
public final class NeoForgeBench {
    private enum Phase { TITLE, WORLD, TELEPORT, LOAD, SETTLE, FLY, DONE }

    private final JsonObject script;
    private final Path out;
    private final List<JsonObject> scenes = new ArrayList<>();
    private Phase phase = Phase.TITLE;
    private int scene = -1;
    private long phaseStart;
    private long lastFrame;
    private long readyCheck;
    private int lastLoaded = -1;
    private long lastChange;
    private final List<Float> frames = new ArrayList<>();

    public NeoForgeBench() {
        String scriptPath = System.getProperty("horizon.bench.script");
        String outPath = System.getProperty("horizon.bench.out");
        if (scriptPath == null || outPath == null) {
            script = null;
            out = null;
            return;
        }
        try {
            script = JsonParser.parseString(Files.readString(Path.of(scriptPath))).getAsJsonObject();
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        out = Path.of(outPath);
        for (var element : script.getAsJsonArray("scenes")) scenes.add(element.getAsJsonObject());
        NeoForge.EVENT_BUS.addListener(RenderFrameEvent.Pre.class, e -> step(Minecraft.getInstance()));
        event("init");
    }

    private void step(Minecraft mc) {
        long now = System.nanoTime();
        float frameMs = lastFrame == 0 ? 0 : (now - lastFrame) / 1e6f;
        lastFrame = now;
        double elapsed = (now - phaseStart) / 1e9;
        if (phase != Phase.TITLE && phase != Phase.DONE && elapsed > script.get("timeout").getAsDouble()
                && phase != Phase.FLY && phase != Phase.SETTLE) {
            JsonObject e = new JsonObject();
            e.addProperty("phase", phase.name());
            event("timeout", e);
            finish(mc);
            return;
        }
        switch (phase) {
            case TITLE -> {
                if (mc.screen instanceof TitleScreen && mc.getOverlay() == null) {
                    event("title", renderer(mc));
                    LevelSettings settings = new LevelSettings(script.get("world").getAsString(), GameType.CREATIVE,
                        false, Difficulty.NORMAL, true, new GameRules(), WorldDataConfiguration.DEFAULT);
                    WorldOptions options = new WorldOptions(script.get("seed").getAsLong(), true, false);
                    enter(Phase.WORLD, now);
                    mc.createWorldOpenFlows().createFreshLevel(script.get("world").getAsString(), settings, options,
                        WorldPresets::createNormalWorldDimensions, mc.screen);
                }
            }
            case WORLD -> {
                if (ready(mc, null, now)) {
                    JsonObject e = new JsonObject();
                    e.addProperty("ready_epoch_ms", epochMs(lastChange));
                    e.addProperty("chunks", lastLoaded);
                    event("world", e);
                    mc.player.connection.sendCommand("weather clear");
                    next(mc, now);
                }
            }
            case TELEPORT -> {
                JsonObject s = scenes.get(scene);
                Vec3 from = vec(s.getAsJsonArray("from"));
                LocalPlayer player = mc.player;
                player.connection.sendCommand("time set 6000");
                player.connection.sendCommand(String.format(java.util.Locale.ROOT, "tp @s %.3f %.3f %.3f %.3f %.3f",
                    from.x, from.y, from.z, yaw(s), s.get("pitch").getAsFloat()));
                event("teleport", named(s));
                enter(Phase.LOAD, now);
            }
            case LOAD -> {
                JsonObject s = scenes.get(scene);
                Vec3 from = vec(s.getAsJsonArray("from"));
                if (mc.player != null && mc.player.position().distanceToSqr(from) < 1024.0) place(mc, s, 0);
                if (ready(mc, from, now)) {
                    JsonObject e = named(s);
                    e.addProperty("load_ms", (lastChange - phaseStart) / 1e6);
                    e.addProperty("chunks", lastLoaded);
                    event("loaded", e);
                    enter(Phase.SETTLE, now);
                }
            }
            case SETTLE -> {
                place(mc, scenes.get(scene), 0);
                if (elapsed >= script.get("settle").getAsDouble()) {
                    frames.clear();
                    event("fly", named(scenes.get(scene)));
                    enter(Phase.FLY, now);
                }
            }
            case FLY -> {
                frames.add(frameMs);
                double t = Math.min(1.0, elapsed / script.get("seconds").getAsDouble());
                place(mc, scenes.get(scene), t);
                if (t >= 1.0) {
                    JsonObject e = named(scenes.get(scene));
                    JsonArray times = new JsonArray();
                    frames.forEach(times::add);
                    e.add("frame_ms", times);
                    e.addProperty("chunks", mc.level.getChunkSource().getLoadedChunksCount());
                    event("flown", e);
                    next(mc, now);
                }
            }
            case DONE -> { }
        }
    }

    private void next(Minecraft mc, long now) {
        scene++;
        if (scene >= scenes.size()) {
            finish(mc);
        } else {
            enter(Phase.TELEPORT, now);
        }
    }

    private void finish(Minecraft mc) {
        phase = Phase.DONE;
        event("done");
        mc.stop();
    }

    private void enter(Phase next, long now) {
        phase = next;
        phaseStart = now;
        lastLoaded = -1;
        lastChange = now;
    }

    /** As VanillaBench.ready: spawned (near {@code at}), no screen, every chunk around, nothing to compile, 1 s stable. */
    private boolean ready(Minecraft mc, Vec3 at, long now) {
        if (now - readyCheck < 10_000_000L) return false;
        readyCheck = now;
        if (mc.level == null || mc.player == null || mc.screen != null || mc.getOverlay() != null
                || (at != null && mc.player.position().distanceToSqr(at) > 4.0)) {
            lastChange = now;
            lastLoaded = -1;
            return false;
        }
        int loaded = mc.level.getChunkSource().getLoadedChunksCount();
        if (loaded != lastLoaded || !mc.levelRenderer.hasRenderedAllSections() || !chunksAround(mc)) {
            lastLoaded = loaded;
            lastChange = now;
            return false;
        }
        return now - lastChange >= 1_000_000_000L;
    }

    private static boolean chunksAround(Minecraft mc) {
        int r = Math.max(1, mc.options.getEffectiveRenderDistance() - 1);
        var center = mc.player.chunkPosition();
        var chunks = mc.level.getChunkSource();
        for (int dx = -r; dx <= r; dx++) {
            for (int dz = -r; dz <= r; dz++) {
                if (dx * dx + dz * dz <= r * r && !chunks.hasChunk(center.x + dx, center.z + dz)) return false;
            }
        }
        return true;
    }

    private static void place(Minecraft mc, JsonObject s, double t) {
        LocalPlayer player = mc.player;
        if (player == null) return;
        Vec3 a = vec(s.getAsJsonArray("from")), b = vec(s.getAsJsonArray("to"));
        Vec3 at = a.add(b.subtract(a).scale(t));
        player.setPos(at.x, at.y, at.z);
        player.setYRot(yaw(s));
        player.setXRot(s.get("pitch").getAsFloat());
        player.setOldPosAndRot();
        player.setDeltaMovement(Vec3.ZERO);
        player.fallDistance = 0;
        if (!player.getAbilities().flying) {
            player.getAbilities().flying = true;
            player.onUpdateAbilities();
        }
    }

    private static float yaw(JsonObject s) {
        Vec3 a = vec(s.getAsJsonArray("from")), b = vec(s.getAsJsonArray("to"));
        return (float) Math.toDegrees(Math.atan2(-(b.x - a.x), b.z - a.z));
    }

    private static Vec3 vec(JsonArray a) {
        return new Vec3(a.get(0).getAsDouble(), a.get(1).getAsDouble(), a.get(2).getAsDouble());
    }

    private static JsonObject named(JsonObject s) {
        JsonObject e = new JsonObject();
        e.addProperty("scene", s.get("name").getAsString());
        return e;
    }

    private static JsonObject renderer(Minecraft mc) {
        JsonObject e = new JsonObject();
        e.addProperty("gpu", GlUtil.getRenderer());
        e.addProperty("vendor", GlUtil.getVendor());
        e.addProperty("backend", "OpenGL");
        e.addProperty("driver", GlUtil.getOpenGLVersion());
        JsonArray size = new JsonArray();
        size.add(mc.getWindow().getWidth());
        size.add(mc.getWindow().getHeight());
        e.add("size", size);
        e.addProperty("render_distance", mc.options.renderDistance().get());
        e.addProperty("simulation_distance", mc.options.simulationDistance().get());
        return e;
    }

    private static long epochMs(long nanos) {
        return System.currentTimeMillis() - (System.nanoTime() - nanos) / 1_000_000L;
    }

    private void event(String name) {
        event(name, new JsonObject());
    }

    private void event(String name, JsonObject e) {
        e.addProperty("event", name);
        e.addProperty("epoch_ms", System.currentTimeMillis());
        try {
            Files.writeString(out, e + "\n", StandardCharsets.UTF_8, StandardOpenOption.CREATE, StandardOpenOption.APPEND);
        } catch (IOException error) {
            throw new UncheckedIOException(error);
        }
    }
}
