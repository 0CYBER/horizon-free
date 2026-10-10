package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.AttackIndicatorStatus;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.gui.contextualbar.ExperienceBar;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.tags.FluidTags;
import net.minecraft.tags.TagKey;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.Util;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.HumanoidArm;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.food.FoodData;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.material.Fluid;
import org.joml.Matrix3x2f;
import org.joml.Matrix3x2fStack;

/**
 * Original oracle for the in-game HUD decorations: runs vanilla's own Hud
 * methods (extractItemHotbar, extractPlayerHealth, extractVehicleHealth,
 * extractCrosshair) and ExperienceBar / ContextualBar on a stand-in client
 * (Unsafe-allocated Minecraft, options, game mode, window and level) and a
 * player whose state getters answer from the case, and records every
 * GuiGraphicsExtractor blit, text and item call.
 *
 * Input lines: case JSON. Output lines: {case, draws:[...]} or {case, error}.
 */
public final class VanillaHudOracle {
    static Object unsafe;
    static Method allocate;
    static JsonObject c;
    static IdentityHashMap<ItemStack, Integer> slots = new IdentityHashMap<>();

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
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : t == float.class ? "putFloat" : t == long.class ? "putLong" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, target, offset, value);
    }

    static float f(String k, float d) { return c.has(k) && !c.get(k).isJsonNull() ? c.get(k).getAsFloat() : d; }
    static int i(String k, int d) { return c.has(k) && !c.get(k).isJsonNull() ? c.get(k).getAsInt() : d; }
    static boolean b(String k) { return c.has(k) && c.get(k).getAsBoolean(); }

    /** A player whose HUD-facing getters answer from the case. */
    public static final class Actor extends LocalPlayer {
        Level level;
        Entity vehicle;
        FoodData food;
        ItemStack offhand = ItemStack.EMPTY;
        private Actor() { super(null, null, null, null, null, null, false, null, null); }
        @Override public float getHealth() { return f("health", 20); }
        @Override public double getAttributeValue(Holder<Attribute> a) { return a == Attributes.MAX_HEALTH ? f("maxHealth", 20) : 0.0; }
        @Override public float getAbsorptionAmount() { return f("absorption", 0); }
        @Override public int getArmorValue() { return i("armor", 0); }
        @Override public FoodData getFoodData() { return food; }
        @Override public boolean hasEffect(Holder<MobEffect> effect) {
            for (var e : c.getAsJsonArray("effects")) if (effect.is(Identifier.parse(e.getAsString()))) return true;
            return false;
        }
        @Override public boolean isFullyFrozen() { return b("frozen"); }
        @Override public int getAirSupply() { return i("air", 300); }
        @Override public int getMaxAirSupply() { return i("maxAir", 300); }
        @Override public boolean isEyeInFluid(TagKey<Fluid> tag) { return tag == FluidTags.WATER && b("underWater"); }
        @Override public Entity getVehicle() { return vehicle; }
        @Override public ItemStack getOffhandItem() { return offhand; }
        @Override public HumanoidArm getMainArm() { return b("rightHanded") ? HumanoidArm.RIGHT : HumanoidArm.LEFT; }
        @Override public float getAttackStrengthScale(float a) { return f("attackScale", 1); }
        @Override public Level level() { return level; }
        @Override public boolean isSpectator() { return false; }
        @Override public void playSound(net.minecraft.sounds.SoundEvent sound, float volume, float pitch) { }
    }

    /** A ridden mount answering health from the case. */
    public static final class Mount extends net.minecraft.world.entity.animal.equine.Horse {
        private Mount() { super(null, null); }
        @Override public float getHealth() { return f("vehicleHealth", 0); }
        @Override public double getAttributeValue(Holder<Attribute> a) { return a == Attributes.MAX_HEALTH ? f("vehicleMaxHealth", 0) : 0.0; }
        @Override public boolean showVehicleHealth() { return true; }
    }

    /** Records GuiGraphicsExtractor calls. */
    public static final class Capture extends GuiGraphicsExtractor {
        JsonArray draws;
        Matrix3x2fStack stack;
        private Capture() { super(null, null, 0, 0); }
        @Override public int guiWidth() { return i("width", 0); }
        @Override public int guiHeight() { return i("height", 0); }
        @Override public Matrix3x2fStack pose() { return stack; }
        @Override public void nextStratum() { }
        void sprite(Identifier id, int x, int y, int w, int h, int[] region, int color) {
            var o = new JsonObject();
            o.addProperty("type", "sprite");
            o.addProperty("sprite", id.getPath());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("w", w); o.addProperty("h", h);
            if (region != null) {
                var r = new JsonArray();
                for (int v : region) r.add(v);
                o.add("region", r);
            }
            o.addProperty("color", color);
            draws.add(o);
        }
        @Override public void blitSprite(RenderPipeline p, Identifier id, int x, int y, int w, int h, int color) { sprite(id, x, y, w, h, null, color); }
        @Override public void blitSprite(RenderPipeline p, Identifier id, int sw, int sh, int u, int v, int x, int y, int w, int h, int color) {
            sprite(id, x, y, w, h, new int[] {sw, sh, u, v}, color);
        }
        @Override public void text(Font font, FormattedCharSequence str, int x, int y, int color, boolean shadow) {
            var o = new JsonObject();
            o.addProperty("type", "text");
            var sb = new StringBuilder();
            str.accept((index, style, cp) -> { sb.appendCodePoint(cp); return true; });
            o.addProperty("text", sb.toString());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("color", color); o.addProperty("shadow", shadow);
            draws.add(o);
        }
        @Override public void item(LivingEntity owner, ItemStack stack, int x, int y, int seed) {
            var o = new JsonObject();
            o.addProperty("type", "item");
            o.addProperty("slot", slots.getOrDefault(stack, -1));
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("seed", seed);
            var m = new Matrix3x2f(stack.isEmpty() ? new Matrix3x2f() : this.stack);
            var a = new JsonArray();
            for (float v : new float[] {m.m00, m.m01, m.m10, m.m11, m.m20, m.m21}) a.add(v);
            o.add("matrix", a);
            draws.add(o);
        }
        @Override public void itemDecorations(Font font, ItemStack stack, int x, int y, String count) { }
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        Font font = ParityFont.create(ParityFont.resources());
        Method hotbar = Hud.class.getDeclaredMethod("extractItemHotbar", GuiGraphicsExtractor.class, DeltaTracker.class);
        Method health = Hud.class.getDeclaredMethod("extractPlayerHealth", GuiGraphicsExtractor.class);
        Method vehicle = Hud.class.getDeclaredMethod("extractVehicleHealth", GuiGraphicsExtractor.class);
        Method crosshair = Hud.class.getDeclaredMethod("extractCrosshair", GuiGraphicsExtractor.class, DeltaTracker.class);
        crosshair.setAccessible(true);
        for (Method m : new Method[] {hotbar, health, vehicle}) m.setAccessible(true);
        Method level = Class.forName("net.minecraft.client.gui.contextualbar.ContextualBar")
            .getDeclaredMethod("extractExperienceLevel", GuiGraphicsExtractor.class, Font.class, int.class);
        level.setAccessible(true);
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            result.add("case", c);
            try {
                slots.clear();
                var mc = alloc(Minecraft.class);
                var actor = alloc(Actor.class);
                var clientLevel = alloc(ClientLevel.class);
                var levelData = alloc(ClientLevel.ClientLevelData.class);
                put(levelData, ClientLevel.ClientLevelData.class, "hardcore", b("hardcore"));
                put(clientLevel, ClientLevel.class, "clientLevelData", levelData);
                actor.level = clientLevel;
                actor.offhand = ItemStack.EMPTY;
                actor.food = new FoodData();
                actor.food.setFoodLevel(i("food", 20));
                actor.food.setSaturation(f("saturation", 5));
                actor.damageCooldownTime = i("damageCooldown", 0);
                actor.experienceProgress = f("xpProgress", 0);
                actor.experienceLevel = i("xpLevel", 0);
                var inventory = new Inventory(actor, new EntityEquipment());
                put(actor, net.minecraft.world.entity.player.Player.class, "inventory", inventory);
                inventory.setSelectedSlot(i("selected", 0));
                var stacks = c.getAsJsonArray("slots");
                var items = new net.minecraft.world.item.Item[] {Items.STONE, Items.DIAMOND_SWORD, Items.APPLE, Items.TORCH, Items.BOW,
                    Items.OAK_LOG, Items.BREAD, Items.SHIELD, Items.COMPASS};
                for (int n = 0; n < 9; n++) {
                    if (stacks.get(n).isJsonNull()) continue;
                    var stack = new ItemStack(items[n]);
                    stack.setPopTime(stacks.get(n).getAsInt());
                    inventory.setItem(n, stack);
                    slots.put(stack, n);
                }
                if (c.has("offhand") && !c.get("offhand").isJsonNull()) {
                    actor.offhand = new ItemStack(Items.TOTEM_OF_UNDYING);
                    actor.offhand.setPopTime(c.get("offhand").getAsInt());
                    slots.put(actor.offhand, 40);
                }
                if (c.has("vehicleMaxHealth")) actor.vehicle = alloc(Mount.class);
                put(mc, Minecraft.class, "player", actor);
                var camera = alloc(net.minecraft.client.Camera.class);
                put(camera, net.minecraft.client.Camera.class, "entity", actor);
                var renderer = alloc(net.minecraft.client.renderer.GameRenderer.class);
                put(renderer, net.minecraft.client.renderer.GameRenderer.class, "mainCamera", camera);
                put(mc, Minecraft.class, "gameRenderer", renderer);
                put(mc, Minecraft.class, "font", font);
                var gameMode = alloc(MultiPlayerGameMode.class);
                put(gameMode, MultiPlayerGameMode.class, "localPlayerMode", GameType.byName(c.get("gameMode").getAsString()));
                put(mc, Minecraft.class, "gameMode", gameMode);
                var options = alloc(Options.class);
                var indicator = alloc(OptionInstance.class);
                put(indicator, OptionInstance.class, "value", AttackIndicatorStatus.valueOf(c.get("attackIndicator").getAsString()));
                put(options, Options.class, "attackIndicator", indicator);
                put(mc, Minecraft.class, "options", options);
                var window = alloc(com.mojang.blaze3d.platform.Window.class);
                put(window, com.mojang.blaze3d.platform.Window.class, "guiScaledWidth", i("width", 0));
                put(window, com.mojang.blaze3d.platform.Window.class, "guiScaledHeight", i("height", 0));
                put(mc, Minecraft.class, "window", window);
                var hud = alloc(Hud.class);
                put(hud, Hud.class, "minecraft", mc);
                put(hud, Hud.class, "random", net.minecraft.util.RandomSource.create());
                put(hud, Hud.class, "tickCount", i("tick", 0));
                put(hud, Hud.class, "lastHealth", i("lastHealth", 0));
                put(hud, Hud.class, "displayHealth", i("displayHealth", 0));
                put(hud, Hud.class, "lastHealthTime", Util.getMillis() - (long) i("lastHealthAgo", 0));
                put(hud, Hud.class, "healthBlinkTime", (long) i("healthBlinkTime", 0));
                var capture = alloc(Capture.class);
                capture.draws = new JsonArray();
                capture.stack = new Matrix3x2fStack(16);
                float partial = f("partial", 0);
                DeltaTracker delta = (DeltaTracker) Proxy.newProxyInstance(DeltaTracker.class.getClassLoader(), new Class<?>[] {DeltaTracker.class},
                    (proxy, method, a) -> method.getReturnType() == float.class ? partial : null);
                var mode = GameType.byName(c.get("gameMode").getAsString());
                if (c.has("crosshair") && c.get("crosshair").getAsBoolean()) {
                    put(options, Options.class, "cameraType", net.minecraft.client.CameraType.FIRST_PERSON);
                    var debug = alloc(net.minecraft.client.gui.components.debug.DebugScreenEntryList.class);
                    put(debug, net.minecraft.client.gui.components.debug.DebugScreenEntryList.class, "currentlyEnabled", new ArrayList<Identifier>());
                    put(mc, Minecraft.class, "debugEntries", debug);
                    crosshair.invoke(hud, capture, delta);
                }
                if (mode != GameType.SPECTATOR) hotbar.invoke(hud, capture, delta);
                if (mode.isSurvival()) health.invoke(hud, capture);
                vehicle.invoke(hud, capture);
                if (c.has("experienceBar") && c.get("experienceBar").getAsBoolean()) new ExperienceBar(mc).extractBackground(capture, delta);
                if (mode.isSurvival() && actor.experienceLevel > 0) level.invoke(null, capture, font, actor.experienceLevel);
                result.add("draws", capture.draws);
            } catch (Throwable error) {
                var cause = error instanceof InvocationTargetException && error.getCause() != null ? error.getCause() : error;
                var sw = new java.io.StringWriter();
                cause.printStackTrace(new java.io.PrintWriter(sw));
                result.addProperty("error", sw.toString().lines().limit(8).reduce("", (x, y) -> x + y + " | "));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
