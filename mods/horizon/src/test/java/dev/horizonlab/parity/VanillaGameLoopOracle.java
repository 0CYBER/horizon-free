package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.FramerateLimitTracker;
import com.mojang.blaze3d.platform.InputConstants;
import com.mojang.blaze3d.platform.Window;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.*;
import net.minecraft.client.*;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.tutorial.Tutorial;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.ServerTickRateManager;
import net.minecraft.server.players.PlayerList;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.entity.player.Inventory;

/**
 * Drives the real DeltaTracker.Timer, ServerTickRateManager, Window.calculateScale,
 * FramerateLimitTracker, MouseHandler (move, scroll, turnPlayer), Entity.turn and
 * ToggleKeyMapping with the input sequences in a JSON-lines file and prints
 * every observable bit-exactly. Hosts are uninitialised shells (Unsafe) that
 * carry only the fields the exercised code reads.
 */
public final class VanillaGameLoopOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static float f(JsonElement v) {
        return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(), 16));
    }

    static double d(JsonElement v) {
        return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(), 16));
    }

    static String fb(float v) {
        return String.format("%08x", Float.floatToRawIntBits(v) == 0x7fc00000 || Float.isNaN(v) ? 0x7fc00000 : Float.floatToRawIntBits(v));
    }

    static String db(double v) {
        return String.format("%016x", Double.isNaN(v) ? 0x7ff8000000000000L : Double.doubleToRawLongBits(v));
    }

    // ---- stubs for the server tick manager -------------------------------------------------
    static final List<String> reports = new ArrayList<>();

    static final class StubStack extends CommandSourceStack {
        StubStack() {
            super(null, null, null, null, null, (Component) null, null);
        }

        @Override
        public void sendSuccess(Supplier<Component> message, boolean broadcast) {
            var contents = (TranslatableContents) message.get().getContents();
            reports.add(contents.getArgs()[0] + " " + contents.getArgs()[1]);
        }
    }

    static final class StubPlayers extends PlayerList {
        StubPlayers() {
            super(null, null, null, (net.minecraft.server.notifications.NotificationService) null);
        }

        @Override
        public void broadcastAll(net.minecraft.network.protocol.Packet<?> packet) {
        }
    }

    static StubStack stubStack;
    static StubPlayers stubPlayers;

    static final class StubServer extends MinecraftServer {
        StubServer() {
            super(null, null, null, null, null, null, null, null, null, false, null);
        }

        @Override public PlayerList getPlayerList() { return stubPlayers; }
        @Override public CommandSourceStack createCommandSourceStack() { return stubStack; }
        @Override public void onTickRateChanged() { }
        @Override protected boolean initServer() { return false; }
        @Override public net.minecraft.server.permissions.LevelBasedPermissionSet operatorUserPermissions() { return null; }
        @Override public net.minecraft.server.permissions.PermissionSet getFunctionCompilationPermissions() { return null; }
        @Override public boolean shouldRconBroadcast() { return false; }
        @Override protected net.minecraft.util.debugchart.SampleLogger getTickTimeLogger() { return null; }
        @Override public boolean isTickTimeLoggingEnabled() { return false; }
        @Override public net.minecraft.SystemReport fillServerSystemReport(net.minecraft.SystemReport r) { return r; }
        @Override public boolean isDedicatedServer() { return false; }
        @Override public int getRateLimitPacketsPerSecond() { return 0; }
        @Override public int getCommandSpamThresholdSeconds() { return 0; }
        @Override public int getChatSpamThresholdSeconds() { return 0; }
        @Override public boolean useNativeTransport() { return false; }
        @Override public boolean isPublished() { return false; }
        @Override public int getMaxPlayers() { return 0; }
        @Override public boolean shouldInformAdmins() { return false; }
        @Override public boolean isSingleplayerOwner(net.minecraft.server.players.NameAndId id) { return false; }
    }

    static final class StubPlayer extends LocalPlayer {
        boolean scoping;
        boolean spectator;

        StubPlayer() {
            super(null, null, null, null, null, Input.EMPTY, false, null, null);
        }

        @Override public boolean isScoping() { return scoping; }
        @Override public boolean isSpectator() { return spectator; }
    }

    static Minecraft shell;
    static Options options;

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        shell = make(Minecraft.class);
        field(Minecraft.class, "fixerUpper").set(shell, net.minecraft.util.datafix.DataFixers.getDataFixer());
        field(Minecraft.class, "instance").set(null, shell);
        options = new Options(shell, Files.createTempDirectory("opts").toFile());
        stubStack = make(StubStack.class);
        stubPlayers = make(StubPlayers.class);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                switch (c.get("t").getAsString()) {
                    case "timer" -> timer(c, out);
                    case "rate" -> rate(c, out);
                    case "scale" -> scale(c, out);
                    case "limit" -> limit(c, out);
                    case "mouse" -> mouse(c, out);
                    case "scroll" -> scroll(c, out);
                    case "key" -> key(c, out);
                    default -> throw new IllegalArgumentException(line);
                }
            }
        }
        System.exit(0);
    }

    static void timer(JsonObject c, PrintWriter out) {
        float[] mspt = {0};
        var timer = new DeltaTracker.Timer(f(c.get("tps")), c.get("start").getAsLong(),
            d -> mspt[0] == 0 ? d : Math.max(d, mspt[0]));
        out.println("timer");
        for (var e : c.getAsJsonArray("ops")) {
            var op = e.getAsJsonArray();
            String name = op.get(0).getAsString();
            String result = "";
            switch (name) {
                case "adv" -> {
                    mspt[0] = op.get(2).isJsonNull() ? 0 : f(op.get(2));
                    result = " " + timer.advanceGameTime(op.get(1).getAsLong());
                }
                case "real" -> timer.advanceRealTime(op.get(1).getAsLong());
                case "pause" -> timer.updatePauseState(op.get(1).getAsBoolean());
                case "frozen" -> timer.updateFrozenState(op.get(1).getAsBoolean());
                default -> throw new IllegalArgumentException(name);
            }
            out.println(name + result + " " + fb(timer.getGameTimeDeltaTicks()) + " " + fb(timer.getGameTimeDeltaPartialTick(false))
                + " " + fb(timer.getGameTimeDeltaPartialTick(true)) + " " + fb(timer.getRealtimeDeltaTicks()));
        }
    }

    static void rate(JsonObject c, PrintWriter out) throws Exception {
        var m = make(ServerTickRateManager.class);
        field(ServerTickRateManager.class, "server").set(m, make(StubServer.class));
        // Field initialisers do not run on a shell.
        field(net.minecraft.world.TickRateManager.class, "tickrate").setFloat(m, 20.0F);
        field(net.minecraft.world.TickRateManager.class, "nanosecondsPerTick").setLong(m, 50_000_000L);
        field(net.minecraft.world.TickRateManager.class, "runGameElements").setBoolean(m, true);
        var startTime = field(ServerTickRateManager.class, "sprintTickStartTime");
        var spend = field(ServerTickRateManager.class, "sprintTimeSpend");
        out.println("rate");
        for (var e : c.getAsJsonArray("ops")) {
            var op = e.getAsJsonArray();
            String name = op.get(0).getAsString();
            reports.clear();
            String result = "";
            switch (name) {
                case "rate" -> m.setTickRate(f(op.get(1)));
                case "freeze" -> m.setFrozen(op.get(1).getAsBoolean());
                case "step" -> result = " " + m.stepGameIfPaused(op.get(1).getAsInt());
                case "stopstep" -> result = " " + m.stopStepping();
                case "sprint" -> result = " " + m.requestGameToSprint(op.get(1).getAsInt());
                case "check" -> {
                    result = " " + m.checkShouldSprintThisTick();
                    if (m.runsNormally() && result.equals(" true")) {
                        startTime.setLong(m, op.get(1).getAsLong());
                    } else if (m.runsNormally() && reports.isEmpty()) {
                        // unreachable: the else branch always reports
                    }
                }
                case "end" -> {
                    long before = spend.getLong(m);
                    m.endTickWork();
                    spend.setLong(m, before + (op.get(1).getAsLong() - startTime.getLong(m)));
                }
                case "stopsprint" -> result = " " + m.stopSprinting();
                case "tick" -> m.tick();
                default -> throw new IllegalArgumentException(name);
            }
            out.println(name + result + " | " + fb(m.tickrate()) + " " + m.nanosecondsPerTick() + " " + fb(m.millisecondsPerTick())
                + " " + m.runsNormally() + " " + m.isSteppingForward() + " " + m.frozenTicksToRun() + " " + m.isFrozen()
                + " " + m.isSprinting() + (reports.isEmpty() ? "" : " report " + reports.get(0)));
        }
    }

    static Window window(int w, int h, int sw, int sh) throws Exception {
        Window window = make(Window.class);
        field(Window.class, "framebufferWidth").setInt(window, w);
        field(Window.class, "framebufferHeight").setInt(window, h);
        field(Window.class, "width").setInt(window, sw);
        field(Window.class, "height").setInt(window, sh);
        return window;
    }

    static void scale(JsonObject c, PrintWriter out) throws Exception {
        Window window = window(c.get("w").getAsInt(), c.get("h").getAsInt(), c.get("sw").getAsInt(), c.get("sh").getAsInt());
        int scale = window.calculateScale(c.get("max").getAsInt(), c.get("uni").getAsBoolean());
        window.setGuiScale(scale);
        out.println("scale " + scale + " " + window.getGuiScaledWidth() + " " + window.getGuiScaledHeight() + " "
            + db(MouseHandler.getScaledXPos(window, d(c.get("x")))) + " " + db(MouseHandler.getScaledYPos(window, d(c.get("y")))));
    }

    static void limit(JsonObject c, PrintWriter out) throws Exception {
        Window window = window(1, 1, 1, 1);
        field(Window.class, "iconified").setBoolean(window, c.get("iconified").getAsBoolean());
        field(Window.class, "focused").setBoolean(window, c.get("focused").getAsBoolean());
        Minecraft mc = make(Minecraft.class);
        field(Minecraft.class, "window").set(mc, window);
        Gui gui = make(Gui.class);
        field(Gui.class, "screen").set(gui, c.get("screen").getAsBoolean() ? make(net.minecraft.client.gui.screens.PauseScreen.class) : null);
        field(Gui.class, "overlay").set(gui, c.get("overlay").getAsBoolean() ? make(net.minecraft.client.gui.screens.LoadingOverlay.class) : null);
        field(Minecraft.class, "gui").set(mc, gui);
        field(Minecraft.class, "level").set(mc, c.get("level").getAsBoolean() ? make(net.minecraft.client.multiplayer.ClientLevel.class) : null);
        options.framerateLimit().set(c.get("opt").getAsInt());
        options.inactivityFpsLimit().set(c.get("afk").getAsBoolean() ? InactivityFpsLimit.AFK : InactivityFpsLimit.MINIMIZED);
        var tracker = new FramerateLimitTracker(options, mc);
        field(FramerateLimitTracker.class, "latestInputTime").setLong(tracker, net.minecraft.util.Util.getMillis() - c.get("ago").getAsLong());
        out.println("limit " + tracker.getThrottleReason() + " " + tracker.getFramerateLimit() + " " + tracker.isHeavilyThrottled());
    }

    static MouseHandler handler(Minecraft mc) throws Exception {
        MouseHandler h = new MouseHandler(mc);
        return h;
    }

    static void mouse(JsonObject c, PrintWriter out) throws Exception {
        Window window = window(1, 1, 1, 1);
        field(Window.class, "handle").setLong(window, 1L);
        field(Window.class, "focused").setBoolean(window, c.get("active").getAsBoolean());
        Minecraft mc = make(Minecraft.class);
        field(Minecraft.class, "window").set(mc, window);
        field(Minecraft.class, "options").set(mc, options);
        field(Minecraft.class, "framerateLimitTracker").set(mc, make(FramerateLimitTracker.class));
        field(Minecraft.class, "tutorial").set(mc, make(Tutorial.class));
        StubPlayer player = make(StubPlayer.class);
        player.scoping = c.get("scoping").getAsBoolean();
        mc.player = player;
        options.sensitivity().set(d(c.get("sens")));
        options.smoothCamera = c.get("smooth").getAsBoolean();
        options.invertMouseX().set(c.get("ix").getAsBoolean());
        options.invertMouseY().set(c.get("iy").getAsBoolean());
        MouseHandler h = handler(mc);
        field(MouseHandler.class, "mouseGrabbed").setBoolean(h, c.get("grabbed").getAsBoolean());
        var turn = MouseHandler.class.getDeclaredMethod("turnPlayer", double.class);
        turn.setAccessible(true);
        var dx = field(MouseHandler.class, "accumulatedDX");
        var dy = field(MouseHandler.class, "accumulatedDY");
        double last = Double.MIN_VALUE;
        out.println("mouse");
        for (var e : c.getAsJsonArray("ops")) {
            var op = e.getAsJsonArray();
            switch (op.get(0).getAsString()) {
                case "move" -> h.onMove(1L, d(op.get(1)), d(op.get(2)), d(op.get(3)), d(op.get(4)));
                case "ignore" -> h.setIgnoreFirstMove();
                case "frame" -> {
                    double time = d(op.get(1));
                    double mousea = time - last;
                    last = time;
                    // MouseHandler.handleAccumulatedMovement without the SDL clock and screens.
                    if (mc.isWindowActive() && h.isMouseGrabbed() && mc.player != null) {
                        turn.invoke(h, mousea);
                    }
                    dx.setDouble(h, 0.0);
                    dy.setDouble(h, 0.0);
                    out.println("frame " + fb(player.getXRot()) + " " + fb(player.getYRot()) + " "
                        + fb(player.xRotO) + " " + fb(player.yRotO));
                }
                default -> throw new IllegalArgumentException(op.toString());
            }
        }
    }

    static void scroll(JsonObject c, PrintWriter out) throws Exception {
        Window window = window(1, 1, 1, 1);
        field(Window.class, "handle").setLong(window, 1L);
        Minecraft mc = make(Minecraft.class);
        field(Minecraft.class, "window").set(mc, window);
        field(Minecraft.class, "options").set(mc, options);
        field(Minecraft.class, "framerateLimitTracker").set(mc, make(FramerateLimitTracker.class));
        field(Minecraft.class, "gui").set(mc, make(Gui.class));
        StubPlayer player = make(StubPlayer.class);
        Inventory inventory = make(Inventory.class);
        inventory.setSelectedSlot(c.get("slot").getAsInt());
        player.spectator = c.has("spectator") && c.get("spectator").getAsBoolean();
        field(net.minecraft.world.entity.player.Player.class, "abilities").set(player, new net.minecraft.world.entity.player.Abilities());
        var hud = make(net.minecraft.client.gui.Hud.class);
        field(net.minecraft.client.gui.Hud.class, "spectatorGui").set(hud,
            make(net.minecraft.client.gui.components.spectator.SpectatorGui.class));
        field(Gui.class, "hud").set(mc.gui, hud);
        field(net.minecraft.world.entity.player.Player.class, "inventory").set(player, inventory);
        mc.player = player;
        options.discreteMouseScroll().set(c.get("discrete").getAsBoolean());
        options.mouseWheelSensitivity().set(d(c.get("sens")));
        MouseHandler h = handler(mc);
        out.println("scroll");
        for (var e : c.getAsJsonArray("ops")) {
            var op = e.getAsJsonArray();
            h.onScroll(1L, d(op.get(0)), d(op.get(1)));
            out.println(inventory.getSelectedSlot() + (player.spectator ? " " + fb(player.getAbilities().getFlyingSpeed()) : ""));
        }
    }

    static int keys = 0;

    static void key(JsonObject c, PrintWriter out) throws Exception {
        AtomicBoolean option = new AtomicBoolean();
        int value = 10000 + keys++;
        KeyMapping mapping = c.get("toggle").getAsBoolean()
            ? new ToggleKeyMapping("key.test" + value, c.get("keyboard").getAsBoolean() ? InputConstants.Type.KEYBOARD : InputConstants.Type.MOUSE,
                value, KeyMapping.Category.MISC, option::get, c.get("restore").getAsBoolean())
            : new KeyMapping("key.test" + value, InputConstants.Type.KEYBOARD, value, KeyMapping.Category.MISC);
        var release = KeyMapping.class.getDeclaredMethod("release");
        release.setAccessible(true);
        var key = c.get("toggle").getAsBoolean() && !c.get("keyboard").getAsBoolean()
            ? InputConstants.Type.MOUSE.getOrCreate(value) : InputConstants.Type.KEYBOARD.getOrCreate(value);
        out.println("key");
        for (var e : c.getAsJsonArray("ops")) {
            var op = e.getAsJsonArray();
            String result = "";
            switch (op.get(0).getAsString()) {
                case "option" -> option.set(op.get(1).getAsBoolean());
                case "down" -> mapping.setDown(op.get(1).getAsBoolean());
                case "release" -> release.invoke(mapping);
                case "restore" -> result = mapping instanceof ToggleKeyMapping t ? " " + t.shouldRestoreStateOnScreenClosed() : " false";
                case "click" -> KeyMapping.click(key);
                case "consume" -> result = " " + mapping.consumeClick();
                case "ingame" -> {
                    var m = KeyMapping.class.getDeclaredMethod("shouldSetOnIngameFocus");
                    m.setAccessible(true);
                    result = " " + m.invoke(mapping);
                }
                default -> throw new IllegalArgumentException(op.toString());
            }
            out.println(op.get(0).getAsString() + result + " " + mapping.isDown());
        }
    }
}
