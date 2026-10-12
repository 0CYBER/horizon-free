package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.authlib.GameProfile;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.serialization.JsonOps;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.gui.components.PlayerTabOverlay;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.player.RemotePlayer;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.network.chat.Style;
import net.minecraft.network.chat.numbers.BlankFormat;
import net.minecraft.network.chat.numbers.FixedFormat;
import net.minecraft.network.chat.numbers.NumberFormat;
import net.minecraft.network.chat.numbers.StyledFormat;
import net.minecraft.resources.Identifier;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.GameType;
import net.minecraft.world.scores.Objective;
import net.minecraft.world.scores.ScoreHolder;
import net.minecraft.world.scores.Scoreboard;
import net.minecraft.world.scores.TeamColor;
import net.minecraft.world.scores.criteria.ObjectiveCriteria;

/**
 * Original oracle for the player list: runs vanilla's own
 * PlayerTabOverlay.extractRenderState frame after frame on a stand-in client
 * (Unsafe-allocated Minecraft, connection with the case's listed PlayerInfos
 * and online mode, level with the case's players, real Scoreboard with the
 * case's teams, list objective and scores, options and Hud ticks) and
 * records every fill, text (as styled runs), sprite and face blit.
 * Visibility changes clear the heart states as setVisible does (without
 * the narration).
 *
 * Input lines: case JSON. Output lines: {case, frames:[[draws]...]} or {case, error}.
 */
public final class VanillaTabListOracle {
    static Object unsafe;
    static Method allocate;
    static JsonArray draws;
    static int width;
    static Scoreboard board;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> type, String name) throws NoSuchFieldException {
        for (Class<?> k = type; k != null; k = k.getSuperclass()) {
            try {
                Field f = k.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
            }
        }
        throw new NoSuchFieldException(name + " on " + type);
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = field(owner, name);
        boolean isStatic = Modifier.isStatic(f.getModifiers());
        long offset = isStatic
            ? (long) unsafe.getClass().getMethod("staticFieldOffset", Field.class).invoke(unsafe, f)
            : (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Object base = isStatic ? unsafe.getClass().getMethod("staticFieldBase", Field.class).invoke(unsafe, f) : target;
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : t == float.class ? "putFloat"
            : t == long.class ? "putLong" : t == double.class ? "putDouble" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, base, offset, value);
    }

    static Object get(Object target, Class<?> owner, String name) throws Exception {
        return field(owner, name).get(target);
    }

    static Component component(JsonElement e) {
        return ComponentSerialization.CODEC.parse(JsonOps.INSTANCE, e).getOrThrow();
    }

    static NumberFormat format(JsonElement e) {
        if (e == null || e.isJsonNull()) return null;
        var o = e.getAsJsonObject();
        return switch (o.get("type").getAsString()) {
            case "blank" -> BlankFormat.INSTANCE;
            case "styled" -> new StyledFormat(Style.Serializer.CODEC.parse(JsonOps.INSTANCE, o.get("style")).getOrThrow());
            case "fixed" -> new FixedFormat(component(o.get("value")));
            default -> throw new IllegalArgumentException(o.toString());
        };
    }

    /** Records GuiGraphicsExtractor calls. */
    public static final class Capture extends GuiGraphicsExtractor {
        private Capture() { super(null, null, 0, 0); }
        @Override public int guiWidth() { return width; }
        @Override public void nextStratum() { }
        @Override public void fill(RenderPipeline p, int x0, int y0, int x1, int y1, int color) {
            var o = new JsonObject();
            o.addProperty("type", "fill");
            o.addProperty("x0", x0); o.addProperty("y0", y0); o.addProperty("x1", x1); o.addProperty("y1", y1);
            o.addProperty("color", color);
            draws.add(o);
        }
        @Override public void blitSprite(RenderPipeline p, Identifier id, int x, int y, int w, int h, int color) {
            var o = new JsonObject();
            o.addProperty("type", "sprite");
            o.addProperty("sprite", id.getPath());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("w", w); o.addProperty("h", h);
            draws.add(o);
        }
        @Override public void blit(RenderPipeline p, Identifier texture, int x, int y, float u, float v, int w, int h, int sw, int sh, int tw, int th, int color) {
            var o = new JsonObject();
            o.addProperty("type", "blit");
            String path = texture.getPath();
            if (path.startsWith("textures/")) path = path.substring(9);
            if (path.endsWith(".png")) path = path.substring(0, path.length() - 4);
            o.addProperty("texture", path);
            var a = new JsonArray();
            for (float n : new float[] {x, y, u, v, w, h, sw, sh, tw, th}) a.add(n);
            o.add("args", a);
            o.addProperty("color", color);
            draws.add(o);
        }
        @Override public void text(Font font, FormattedCharSequence str, int x, int y, int color, boolean shadow) {
            var o = new JsonObject();
            o.addProperty("type", "text");
            var runs = new JsonArray();
            str.accept((index, style, cp) -> {
                var key = new JsonArray();
                key.add(style.getColor() == null ? -1 : style.getColor().getValue());
                key.add(style.isBold()); key.add(style.isItalic()); key.add(style.isUnderlined());
                key.add(style.isStrikethrough()); key.add(style.isObfuscated());
                var last = runs.isEmpty() ? null : runs.get(runs.size() - 1).getAsJsonArray();
                if (last != null && last.get(1).equals(key)) {
                    last.set(0, new JsonPrimitive(last.get(0).getAsString() + new String(Character.toChars(cp))));
                } else {
                    var run = new JsonArray();
                    run.add(new String(Character.toChars(cp)));
                    run.add(key);
                    runs.add(run);
                }
                return true;
            });
            o.add("runs", runs);
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("color", color); o.addProperty("shadow", shadow);
            draws.add(o);
        }
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        Font font = ParityFont.create(ParityFont.resources());
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            JsonObject c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            result.add("case", c);
            try {
                width = c.get("width").getAsInt();
                board = new Scoreboard();
                for (var t : c.getAsJsonArray("teams")) {
                    var o = t.getAsJsonObject();
                    var team = board.addPlayerTeam(o.get("name").getAsString());
                    if (!o.get("color").isJsonNull()) team.setColor(Optional.of(TeamColor.byName(o.get("color").getAsString())));
                    team.setPlayerPrefix(component(o.get("prefix")));
                    team.setPlayerSuffix(component(o.get("suffix")));
                    for (var m : o.getAsJsonArray("members")) board.addPlayerToTeam(m.getAsString(), team);
                }
                Objective objective = null;
                if (!c.get("objective").isJsonNull()) {
                    var o = c.getAsJsonObject("objective");
                    var type = o.get("renderType").getAsString().equals("hearts") ? ObjectiveCriteria.RenderType.HEARTS : ObjectiveCriteria.RenderType.INTEGER;
                    objective = board.addObjective(o.get("name").getAsString(), ObjectiveCriteria.DUMMY, Component.literal("x"), type, false, format(o.get("numberFormat")));
                }
                var mc = alloc(Minecraft.class);
                put(null, Minecraft.class, "instance", mc);
                var level = alloc(ClientLevel.class);
                var levelPlayers = new ArrayList<net.minecraft.client.player.AbstractClientPlayer>();
                put(level, ClientLevel.class, "players", levelPlayers);
                put(mc, Minecraft.class, "level", level);
                put(mc, Minecraft.class, "font", font);
                var options = alloc(Options.class);
                var chatOnly = alloc(OptionInstance.class);
                put(chatOnly, OptionInstance.class, "value", c.get("backgroundForChatOnly").getAsBoolean());
                put(options, Options.class, "backgroundForChatOnly", chatOnly);
                var opacity = alloc(OptionInstance.class);
                put(opacity, OptionInstance.class, "value", c.get("opacity").getAsDouble());
                put(options, Options.class, "textBackgroundOpacity", opacity);
                put(mc, Minecraft.class, "options", options);
                var connection = alloc(ClientPacketListener.class);
                put(connection, ClientPacketListener.class, "scoreboard", board);
                put(level, ClientLevel.class, "connection", connection);
                put(connection, ClientPacketListener.class, "onlineMode", c.get("onlineMode").getAsBoolean());
                var listed = new LinkedHashSet<PlayerInfo>();
                put(connection, ClientPacketListener.class, "listedPlayers", listed);
                var player = alloc(LocalPlayer.class);
                put(player, LocalPlayer.class, "connection", connection);
                put(mc, Minecraft.class, "player", player);
                for (var p : c.getAsJsonArray("players")) {
                    var o = p.getAsJsonObject();
                    var uuid = UUID.fromString(o.get("uuid").getAsString());
                    var profile = new GameProfile(uuid, o.get("name").getAsString());
                    var info = new PlayerInfo(profile, false);
                    put(info, PlayerInfo.class, "gameMode", GameType.byId(o.get("gameMode").getAsInt()));
                    put(info, PlayerInfo.class, "latency", o.get("latency").getAsInt());
                    if (!o.get("displayName").isJsonNull()) info.setTabListDisplayName(component(o.get("displayName")));
                    info.setTabListOrder(o.get("listOrder").getAsInt());
                    info.setShowHat(o.get("showHat").getAsBoolean());
                    var skin = net.minecraft.client.resources.DefaultPlayerSkin.get(uuid);
                    put(info, PlayerInfo.class, "skinLookup", (java.util.function.Supplier<?>) () -> skin);
                    listed.add(info);
                    if (o.get("inLevel").getAsBoolean()) {
                        var entity = alloc(RemotePlayer.class);
                        put(entity, Player.class, "gameProfile", profile);
                        put(entity, net.minecraft.world.entity.Entity.class, "uuid", uuid);
                        levelPlayers.add(entity);
                    }
                }
                var hud = alloc(Hud.class);
                put(hud, Hud.class, "minecraft", mc);
                var overlay = new PlayerTabOverlay(mc, hud);
                if (!c.get("header").isJsonNull()) overlay.setHeader(component(c.get("header")));
                if (!c.get("footer").isJsonNull()) overlay.setFooter(component(c.get("footer")));
                @SuppressWarnings("unchecked")
                var health = (Map<UUID, ?>) get(overlay, PlayerTabOverlay.class, "healthStates");
                var capture = alloc(Capture.class);
                var frames = new JsonArray();
                for (var f : c.getAsJsonArray("frames")) {
                    var o = f.getAsJsonObject();
                    if (objective != null) {
                        for (var s : o.getAsJsonObject("scores").entrySet()) {
                            var v = s.getValue().getAsJsonObject();
                            var access = board.getOrCreatePlayerScore(ScoreHolder.forNameOnly(s.getKey()), objective);
                            access.set(v.get("value").getAsInt());
                            access.numberFormatOverride(format(v.get("format")));
                        }
                        for (var r : o.getAsJsonArray("reset")) board.resetSinglePlayerScore(ScoreHolder.forNameOnly(r.getAsString()), objective);
                    }
                    put(hud, Hud.class, "tickCount", o.get("tick").getAsInt());
                    boolean visible = o.get("visible").getAsBoolean();
                    if ((boolean) get(overlay, PlayerTabOverlay.class, "visible") != visible) {
                        health.clear();
                        put(overlay, PlayerTabOverlay.class, "visible", visible);
                    }
                    draws = new JsonArray();
                    if (visible) overlay.extractRenderState(capture, width, board, objective);
                    frames.add(draws);
                }
                result.add("frames", frames);
            } catch (Throwable error) {
                var cause = error instanceof InvocationTargetException && error.getCause() != null ? error.getCause() : error;
                var sw = new java.io.StringWriter();
                cause.printStackTrace(new java.io.PrintWriter(sw));
                result.addProperty("error", sw.toString().lines().limit(10).reduce("", (x, y) -> x + y + " | "));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
