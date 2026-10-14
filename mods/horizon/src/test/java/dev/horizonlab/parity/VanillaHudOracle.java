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
 * Oracle for the in-game HUD: runs vanilla's own Hud methods (hotbar,
 * crosshair, health, mount, experience, effects, titles and the action bar,
 * the selected item name, the sidebar, camera and sleep overlays and
 * Hud.tick), BossHealthOverlay, JumpableVehicleBar, ChatComponent and
 * GuiGraphicsExtractor.itemDecorations on a stand-in client (Unsafe-allocated
 * Minecraft, options, game mode, window and level) and a player whose state
 * getters answer from the case, and records every GuiGraphicsExtractor blit,
 * fill, text and item call. See tools/test_hud_parity.py for the case kinds.
 *
 * Input lines: case JSON. Output lines: {case, draws:[...], state...} or {case, error}.
 */
public final class VanillaHudOracle {
    static Object unsafe;
    static Method allocate;
    static JsonObject c;
    static Font FONT;
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

    static void putStatic(Class<?> owner, String name, Object value) throws Exception {
        Field f = field(owner, name);
        var u = unsafe.getClass();
        Object base = u.getMethod("staticFieldBase", Field.class).invoke(unsafe, f);
        long offset = (long) u.getMethod("staticFieldOffset", Field.class).invoke(unsafe, f);
        u.getMethod("putObject", Object.class, long.class, Object.class).invoke(unsafe, base, offset, value);
    }

    static float f(String k, float d) { return c.has(k) && !c.get(k).isJsonNull() ? c.get(k).getAsFloat() : d; }
    static int i(String k, int d) { return c.has(k) && !c.get(k).isJsonNull() ? c.get(k).getAsInt() : d; }
    static boolean b(String k) { return c.has(k) && c.get(k).getAsBoolean(); }
    static net.minecraft.network.chat.Component comp(JsonElement e) {
        return net.minecraft.network.chat.ComponentSerialization.CODEC.parse(com.mojang.serialization.JsonOps.INSTANCE, e).getOrThrow();
    }
    static <T> OptionInstance<T> option(T value) throws Exception {
        @SuppressWarnings("unchecked") OptionInstance<T> o = alloc(OptionInstance.class);
        put(o, OptionInstance.class, "value", value);
        return o;
    }
    static net.minecraft.network.chat.numbers.NumberFormat numberFormat(JsonElement e) {
        if (e == null || e.isJsonNull()) return null;
        var o = e.getAsJsonObject();
        return switch (o.get("type").getAsString()) {
            case "blank" -> net.minecraft.network.chat.numbers.BlankFormat.INSTANCE;
            case "fixed" -> new net.minecraft.network.chat.numbers.FixedFormat(comp(o.get("value")));
            case "styled" -> new net.minecraft.network.chat.numbers.StyledFormat(
                net.minecraft.network.chat.Style.Serializer.MAP_CODEC.codec().parse(com.mojang.serialization.JsonOps.INSTANCE, o.get("style")).getOrThrow());
            default -> throw new IllegalArgumentException(o.toString());
        };
    }
    static Method hudMethod(String name, Class<?>... params) throws Exception {
        Method m = Hud.class.getDeclaredMethod(name, params);
        m.setAccessible(true);
        return m;
    }

    /** A player whose HUD-facing getters answer from the case. */
    public static final class Actor extends LocalPlayer {
        Level level;
        Entity vehicle;
        FoodData food;
        ItemStack offhand = ItemStack.EMPTY;
        List<net.minecraft.world.effect.MobEffectInstance> active = new ArrayList<>();
        private Actor() { super(null, null, null, null, null, null, false, null, null); }
        @Override public float getHealth() { return f("health", 20); }
        @Override public double getAttributeValue(Holder<Attribute> a) { return a == Attributes.MAX_HEALTH ? f("maxHealth", 20) : 0.0; }
        @Override public float getAbsorptionAmount() { return f("absorption", 0); }
        @Override public int getArmorValue() { return i("armor", 0); }
        @Override public FoodData getFoodData() { return food; }
        @Override public boolean hasEffect(Holder<MobEffect> effect) {
            if (c.has("effects")) for (var e : c.getAsJsonArray("effects")) if (effect.is(Identifier.parse(e.getAsString()))) return true;
            return false;
        }
        @Override public java.util.Collection<net.minecraft.world.effect.MobEffectInstance> getActiveEffects() { return active; }
        @Override public boolean isFullyFrozen() { return b("frozen"); }
        @Override public int getAirSupply() { return i("air", 300); }
        @Override public int getMaxAirSupply() { return i("maxAir", 300); }
        @Override public boolean isEyeInFluid(TagKey<Fluid> tag) { return tag == FluidTags.WATER && b("underWater"); }
        @Override public Entity getVehicle() { return vehicle; }
        @Override public ItemStack getOffhandItem() { return offhand; }
        @Override public HumanoidArm getMainArm() { return b("rightHanded") ? HumanoidArm.RIGHT : HumanoidArm.LEFT; }
        @Override public float getAttackStrengthScale(float a) { return f("attackScale", 1); }
        @Override public float getCurrentItemAttackStrengthDelay() { return f("attackDelay", 5); }
        @Override public ItemStack getActiveItem() { return ItemStack.EMPTY; }
        @Override public Level level() { return level; }
        @Override public boolean isSpectator() { return false; }
        @Override public net.minecraft.world.entity.PlayerRideableJumping jumpableVehicle() {
            return (net.minecraft.world.entity.PlayerRideableJumping) Proxy.newProxyInstance(Actor.class.getClassLoader(),
                new Class<?>[] {net.minecraft.world.entity.PlayerRideableJumping.class},
                (proxy, method, a) -> method.getName().equals("getJumpCooldown") ? i("cooldown", 0)
                    : method.getReturnType() == boolean.class ? false : method.getReturnType() == float.class ? 0.0F : null);
        }
        @Override public float getJumpRidingScale() { return f("jumpScale", 0); }
        @Override public ItemStack getItemBySlot(net.minecraft.world.entity.EquipmentSlot slot) {
            return slot == net.minecraft.world.entity.EquipmentSlot.HEAD && c.has("head") && !c.get("head").isJsonNull()
                ? new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.withDefaultNamespace(c.get("head").getAsString()))) : ItemStack.EMPTY;
        }
        @Override public int getTicksFrozen() { return i("ticksFrozen", 0); }
        @Override public boolean isScoping() { return b("scoping"); }
        @Override public float getEffectBlendFactor(Holder<MobEffect> effect, float partial) {
            return effect.is(net.minecraft.world.effect.MobEffects.NAUSEA) ? f("nausea", 0) : 0.0F;
        }
        @Override public int getSleepTimer() { return i("sleepTimer", 0); }
        @Override public void playSound(net.minecraft.sounds.SoundEvent sound, float volume, float pitch) { }
    }

    /** A ridden mount answering health from the case. */
    public static final class Mount extends net.minecraft.world.entity.animal.equine.Horse {
        private Mount() { super(null, null); }
        @Override public float getHealth() { return f("vehicleHealth", 0); }
        @Override public double getAttributeValue(Holder<Attribute> a) { return a == Attributes.MAX_HEALTH ? f("vehicleMaxHealth", 0) : 0.0; }
        @Override public boolean showVehicleHealth() { return true; }
    }

    /** Minecraft.crosshairPickEntity: a living entity, alive as the case says. */
    public static final class Target extends net.minecraft.world.entity.animal.equine.Horse {
        private Target() { super(null, null); }
        @Override public boolean isAlive() { return b("target"); }
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
        /** Adds the pose when it is not the identity. */
        JsonObject posed(JsonObject o, org.joml.Matrix3x2fc m) {
            if (!m.equals(new Matrix3x2f())) {
                var a = new JsonArray();
                for (float v : new float[] {m.m00(), m.m01(), m.m10(), m.m11(), m.m20(), m.m21()}) a.add(v);
                o.add("matrix", a);
            }
            draws.add(o);
            return o;
        }
        void sprite(RenderPipeline p, Identifier id, int x, int y, int w, int h, int[] region, int color) {
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
            o.addProperty("pipeline", p.getLocation().getPath());
            posed(o, stack);
        }
        @Override public void blitSprite(RenderPipeline p, Identifier id, int x, int y, int w, int h, int color) { sprite(p, id, x, y, w, h, null, color); }
        @Override public void blitSprite(RenderPipeline p, Identifier id, int sw, int sh, int u, int v, int x, int y, int w, int h, int color) {
            sprite(p, id, x, y, w, h, new int[] {sw, sh, u, v}, color);
        }
        @Override public void blitSprite(RenderPipeline p, net.minecraft.client.renderer.texture.TextureAtlasSprite s, int x, int y, int w, int h, int color) {
            var o = new JsonObject();
            o.addProperty("type", "atlas");
            o.addProperty("sprite", s.contents().name().getPath());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("w", w); o.addProperty("h", h);
            o.addProperty("color", color);
            posed(o, stack);
        }
        @Override public void blit(RenderPipeline p, Identifier texture, int x, int y, float u, float v, int w, int h, int srcW, int srcH, int texW, int texH, int color) {
            var o = new JsonObject();
            o.addProperty("type", "blit");
            o.addProperty("texture", texture.getPath());
            o.addProperty("pipeline", p.getLocation().getPath());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("w", w); o.addProperty("h", h);
            var uv = new JsonArray(); uv.add(u); uv.add(v); o.add("uv", uv);
            var src = new JsonArray(); src.add(srcW); src.add(srcH); o.add("src", src);
            var tex = new JsonArray(); tex.add(texW); tex.add(texH); o.add("tex", tex);
            o.addProperty("color", color);
            posed(o, stack);
        }
        @Override public void fill(RenderPipeline p, int x0, int y0, int x1, int y1, int color) {
            var o = new JsonObject();
            o.addProperty("type", "fill");
            o.addProperty("x0", Math.min(x0, x1)); o.addProperty("y0", Math.min(y0, y1));
            o.addProperty("x1", Math.max(x0, x1)); o.addProperty("y1", Math.max(y0, y1));
            o.addProperty("color", color);
            posed(o, stack);
        }
        void text(FormattedCharSequence str, int x, int y, int color, boolean shadow, org.joml.Matrix3x2fc pose) {
            var o = new JsonObject();
            o.addProperty("type", "text");
            var sb = new StringBuilder();
            var runs = new JsonArray();
            boolean[] styled = {false};
            str.accept((index, style, cp) -> {
                sb.appendCodePoint(cp);
                Integer rgb = style.getColor() == null ? null : style.getColor().getValue();
                boolean bold = style.isBold(), italic = style.isItalic();
                styled[0] |= rgb != null || bold || italic;
                JsonArray last = runs.isEmpty() ? null : runs.get(runs.size() - 1).getAsJsonArray();
                if (last != null && java.util.Objects.equals(last.get(1).isJsonNull() ? null : last.get(1).getAsInt(), rgb)
                    && last.get(2).getAsBoolean() == bold && last.get(3).getAsBoolean() == italic) {
                    last.set(0, new JsonPrimitive(last.get(0).getAsString() + new String(Character.toChars(cp))));
                } else {
                    var run = new JsonArray();
                    run.add(new String(Character.toChars(cp)));
                    if (rgb == null) run.add(JsonNull.INSTANCE); else run.add(rgb);
                    run.add(bold); run.add(italic);
                    runs.add(run);
                }
                return true;
            });
            o.addProperty("text", sb.toString());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("color", color); o.addProperty("shadow", shadow);
            if (styled[0]) o.add("runs", runs);
            posed(o, pose);
        }
        @Override public void text(Font font, FormattedCharSequence str, int x, int y, int color, boolean shadow) {
            if (net.minecraft.util.ARGB.alpha(color) != 0) text(str, x, y, color, shadow, stack);
        }
        /** The chat's text collector: text at the parameters' pose and opacity. */
        @Override public net.minecraft.client.gui.ActiveTextCollector textRenderer(GuiGraphicsExtractor.HoveredTextEffects effects, java.util.function.Consumer<net.minecraft.network.chat.Style> consumer) {
            var self = this;
            return new net.minecraft.client.gui.ActiveTextCollector() {
                Parameters parameters = new Parameters(new Matrix3x2f(stack), 1.0F, null);
                @Override public Parameters defaultParameters() { return parameters; }
                @Override public void defaultParameters(Parameters p) { parameters = p; }
                @Override public void accept(net.minecraft.client.gui.TextAlignment alignment, int anchorX, int y, Parameters p, FormattedCharSequence text) {
                    int left = alignment.calculateLeft(anchorX, FONT, text);
                    if (net.minecraft.util.ARGB.as8BitChannel(p.opacity()) != 0)
                        self.text(text, left, y, net.minecraft.util.ARGB.white(p.opacity()), true, p.pose());
                }
                @Override public void acceptScrolling(net.minecraft.network.chat.Component m, int cx, int l, int r, int t, int b, Parameters p) { }
            };
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
        /** Off for the hotbar cases (Horizon draws the decorations with the item). */
        boolean realDecorations;
        @Override public void itemDecorations(Font font, ItemStack stack, int x, int y, String count) {
            if (realDecorations) super.itemDecorations(font, stack, x, y, count);
        }
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        Font font = FONT = ParityFont.create(ParityFont.resources());
        Method hotbar = Hud.class.getDeclaredMethod("extractItemHotbar", GuiGraphicsExtractor.class, DeltaTracker.class);
        Method health = Hud.class.getDeclaredMethod("extractPlayerHealth", GuiGraphicsExtractor.class);
        Method vehicle = Hud.class.getDeclaredMethod("extractVehicleHealth", GuiGraphicsExtractor.class);
        Method crosshair = Hud.class.getDeclaredMethod("extractCrosshair", GuiGraphicsExtractor.class, DeltaTracker.class);
        crosshair.setAccessible(true);
        for (Method m : new Method[] {hotbar, health, vehicle}) m.setAccessible(true);
        Method effects = hudMethod("extractEffects", GuiGraphicsExtractor.class, DeltaTracker.class);
        Method title = hudMethod("extractTitle", GuiGraphicsExtractor.class, DeltaTracker.class);
        Method overlayMessage = hudMethod("extractOverlayMessage", GuiGraphicsExtractor.class, DeltaTracker.class);
        Method selectedName = hudMethod("extractSelectedItemName", GuiGraphicsExtractor.class);
        Method tick = hudMethod("tick");
        Method cameraOverlays = hudMethod("extractCameraOverlays", GuiGraphicsExtractor.class, DeltaTracker.class);
        Method sleep = hudMethod("extractSleepOverlay", GuiGraphicsExtractor.class, DeltaTracker.class);
        Method sidebar = hudMethod("displayScoreboardSidebar", GuiGraphicsExtractor.class, net.minecraft.world.scores.Objective.class);
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
                actor.active = new ArrayList<>();
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
                for (int n = 0; stacks != null && n < 9; n++) {
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
                put(gameMode, MultiPlayerGameMode.class, "localPlayerMode", GameType.byName(c.has("gameMode") ? c.get("gameMode").getAsString() : "survival"));
                put(mc, Minecraft.class, "gameMode", gameMode);
                var options = alloc(Options.class);
                put(options, Options.class, "attackIndicator", option(AttackIndicatorStatus.valueOf(c.has("attackIndicator") ? c.get("attackIndicator").getAsString() : "OFF")));
                put(options, Options.class, "backgroundForChatOnly", option(c.has("chatOnly") ? b("chatOnly") : true));
                put(options, Options.class, "textBackgroundOpacity", option((double) f("textOpacity", 0.5F)));
                put(options, Options.class, "notificationDisplayTime", option(c.has("displayTime") ? c.get("displayTime").getAsDouble() : 1.0));
                put(mc, Minecraft.class, "options", options);
                var window = alloc(com.mojang.blaze3d.platform.Window.class);
                put(window, com.mojang.blaze3d.platform.Window.class, "guiScaledWidth", i("width", 0));
                put(window, com.mojang.blaze3d.platform.Window.class, "guiScaledHeight", i("height", 0));
                put(mc, Minecraft.class, "window", window);
                var gui = alloc(net.minecraft.client.gui.Gui.class);
                put(mc, Minecraft.class, "gui", gui);
                put(mc, Minecraft.class, "demo", b("demo"));
                var hud = alloc(Hud.class);
                put(hud, Hud.class, "minecraft", mc);
                put(hud, Hud.class, "random", net.minecraft.util.RandomSource.create());
                put(hud, Hud.class, "tickCount", i("tick", 0));
                put(hud, Hud.class, "lastHealth", i("lastHealth", 0));
                put(hud, Hud.class, "displayHealth", i("displayHealth", 0));
                put(hud, Hud.class, "lastHealthTime", Util.getMillis() - (long) i("lastHealthAgo", 0));
                put(hud, Hud.class, "healthBlinkTime", (long) i("healthBlinkTime", 0));
                put(hud, Hud.class, "lastToolHighlight", ItemStack.EMPTY);
                var chat = alloc(net.minecraft.client.gui.components.ChatComponent.class);
                put(chat, net.minecraft.client.gui.components.ChatComponent.class, "messageDeletionQueue", new ArrayList<>());
                put(hud, Hud.class, "chat", chat);
                hud.resetTitleTimes();
                var capture = alloc(Capture.class);
                capture.draws = new JsonArray();
                capture.stack = new Matrix3x2fStack(16);
                put(capture, GuiGraphicsExtractor.class, "minecraft", mc);
                put(capture, GuiGraphicsExtractor.class, "pose", capture.stack);
                float partial = f("partial", 0);
                DeltaTracker delta = (DeltaTracker) Proxy.newProxyInstance(DeltaTracker.class.getClassLoader(), new Class<?>[] {DeltaTracker.class},
                    (proxy, method, a) -> method.getReturnType() == float.class ? partial : null);
                var mode = GameType.byName(c.has("gameMode") ? c.get("gameMode").getAsString() : "survival");
                // Hud.tick's vignette brightness reads the camera entity's level: tick without one.
                if (Set.of("title", "selected").contains(c.get("kind").getAsString()))
                    put(camera, net.minecraft.client.Camera.class, "entity", null);
                switch (c.get("kind").getAsString()) {
                    case "decorations" -> {
                        if (c.has("crosshair") && c.get("crosshair").getAsBoolean()) {
                            put(options, Options.class, "cameraType", net.minecraft.client.CameraType.FIRST_PERSON);
                            var debug = alloc(net.minecraft.client.gui.components.debug.DebugScreenEntryList.class);
                            put(debug, net.minecraft.client.gui.components.debug.DebugScreenEntryList.class, "currentlyEnabled", new ArrayList<Identifier>());
                            put(mc, Minecraft.class, "debugEntries", debug);
                            boolean target = c.has("target") && !c.get("target").isJsonNull();
                            put(mc, Minecraft.class, "crosshairPickEntity", target ? alloc(Target.class) : null);
                            crosshair.invoke(hud, capture, delta);
                        }
                        if (mode != GameType.SPECTATOR) hotbar.invoke(hud, capture, delta);
                        if (mode.isSurvival()) health.invoke(hud, capture);
                        vehicle.invoke(hud, capture);
                        if (c.has("experienceBar") && c.get("experienceBar").getAsBoolean()) new ExperienceBar(mc).extractBackground(capture, delta);
                        if (mode.isSurvival() && actor.experienceLevel > 0) level.invoke(null, capture, font, actor.experienceLevel);
                    }
                    case "effects" -> {
                        for (var e : c.getAsJsonArray("active")) {
                            var o = e.getAsJsonObject();
                            var effect = BuiltInRegistries.MOB_EFFECT.get(Identifier.parse(o.get("id").getAsString())).orElseThrow();
                            actor.active.add(new net.minecraft.world.effect.MobEffectInstance(effect, o.get("duration").getAsInt(), 0,
                                o.get("ambient").getAsBoolean(), true, o.get("icon").getAsBoolean()));
                        }
                        effects.invoke(hud, capture, delta);
                    }
                    case "title" -> {
                        for (var e : c.getAsJsonArray("ops")) {
                            var op = e.getAsJsonArray();
                            switch (op.get(0).getAsString()) {
                                case "title" -> hud.setTitle(comp(op.get(1)));
                                case "subtitle" -> hud.setSubtitle(comp(op.get(1)));
                                case "times" -> hud.setTimes(op.get(1).getAsInt(), op.get(2).getAsInt(), op.get(3).getAsInt());
                                case "clear" -> hud.clearTitles();
                                case "reset" -> hud.resetTitleTimes();
                                case "overlay" -> hud.setOverlayMessage(comp(op.get(1)), op.get(2).getAsBoolean());
                                case "tick" -> { for (int n = op.get(1).getAsInt(); n > 0; n--) tick.invoke(hud); }
                                default -> throw new IllegalArgumentException(op.toString());
                            }
                        }
                        overlayMessage.invoke(hud, capture, delta);
                        title.invoke(hud, capture, delta);
                    }
                    case "selected" -> {
                        int slot = i("selected", 0);
                        for (var e : c.getAsJsonArray("steps")) {
                            ItemStack stack = ItemStack.EMPTY;
                            if (!e.isJsonNull()) {
                                var o = e.getAsJsonObject();
                                stack = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.withDefaultNamespace(o.get("item").getAsString())));
                                if (!o.get("custom").isJsonNull()) stack.set(net.minecraft.core.component.DataComponents.CUSTOM_NAME, comp(o.get("custom")));
                            }
                            inventory.setItem(slot, stack);
                            tick.invoke(hud);
                        }
                        selectedName.invoke(hud, capture);
                    }
                    case "boss" -> {
                        var overlay = new net.minecraft.client.gui.components.BossHealthOverlay(mc);
                        @SuppressWarnings("unchecked") var events = (Map<UUID, net.minecraft.client.gui.components.LerpingBossEvent>)
                            field(net.minecraft.client.gui.components.BossHealthOverlay.class, "events").get(overlay);
                        for (var e : c.getAsJsonArray("bars")) {
                            var o = e.getAsJsonObject();
                            var id = UUID.randomUUID();
                            events.put(id, new net.minecraft.client.gui.components.LerpingBossEvent(id, comp(o.get("name")), o.get("progress").getAsFloat(),
                                net.minecraft.world.BossEvent.BossBarColor.values()[o.get("color").getAsInt()],
                                net.minecraft.world.BossEvent.BossBarOverlay.values()[o.get("overlay").getAsInt()], false, false, false));
                        }
                        overlay.extractRenderState(capture);
                    }
                    case "jump" -> new net.minecraft.client.gui.contextualbar.JumpableVehicleBar(mc).extractBackground(capture, delta);
                    case "sidebar" -> {
                        var board = new net.minecraft.world.scores.Scoreboard();
                        var o = c.getAsJsonObject("objective");
                        var objective = board.addObjective("sidebar", net.minecraft.world.scores.criteria.ObjectiveCriteria.DUMMY, comp(o.get("display")),
                            net.minecraft.world.scores.criteria.ObjectiveCriteria.RenderType.INTEGER, false, numberFormat(o.get("format")));
                        for (var e : c.getAsJsonArray("scores")) {
                            var sc = e.getAsJsonObject();
                            var access = board.getOrCreatePlayerScore(net.minecraft.world.scores.ScoreHolder.forNameOnly(sc.get("owner").getAsString()), objective);
                            access.set(sc.get("value").getAsInt());
                            if (!sc.get("display").isJsonNull()) access.display(comp(sc.get("display")));
                            access.numberFormatOverride(numberFormat(sc.get("format")));
                        }
                        for (var e : c.getAsJsonArray("teams")) {
                            var t = e.getAsJsonObject();
                            var team = board.addPlayerTeam(t.get("name").getAsString());
                            team.setPlayerPrefix(comp(t.get("prefix")));
                            team.setPlayerSuffix(comp(t.get("suffix")));
                            team.setColor(t.get("color").isJsonNull() ? Optional.empty()
                                : Optional.of(net.minecraft.world.scores.TeamColor.byName(t.get("color").getAsString())));
                            for (var m : t.getAsJsonArray("members")) board.addPlayerToTeam(m.getAsString(), team);
                        }
                        sidebar.invoke(hud, capture, objective);
                    }
                    case "decor" -> {
                        var stack = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.withDefaultNamespace(c.get("item").getAsString())));
                        stack.setCount(i("count", 1));
                        stack.setDamageValue(i("damage", 0));
                        if (b("unbreakable")) stack.set(net.minecraft.core.component.DataComponents.UNBREAKABLE, net.minecraft.util.Unit.INSTANCE);
                        var cooldowns = new net.minecraft.world.item.ItemCooldowns();
                        put(actor, net.minecraft.world.entity.player.Player.class, "cooldowns", cooldowns);
                        if (c.has("cooldown") && !c.get("cooldown").isJsonNull()) {
                            var cd = c.getAsJsonObject("cooldown");
                            cooldowns.addCooldown(stack, cd.get("duration").getAsInt());
                            for (int n = cd.get("elapsed").getAsInt(); n > 0; n--) cooldowns.tick();
                        }
                        var timer = alloc(DeltaTracker.Timer.class);
                        put(timer, DeltaTracker.Timer.class, "deltaTickResidual", partial);
                        put(mc, Minecraft.class, "deltaTracker", timer);
                        capture.realDecorations = true;
                        capture.itemDecorations(font, stack, i("x", 0), i("y", 0), c.has("countText") ? c.get("countText").getAsString() : null);
                    }
                    case "overlays" -> {
                        put(options, Options.class, "vignette", option(b("vignette")));
                        put(options, Options.class, "cameraType", b("firstPerson") ? net.minecraft.client.CameraType.FIRST_PERSON : net.minecraft.client.CameraType.THIRD_PERSON_BACK);
                        put(options, Options.class, "screenEffectScale", option((double) f("screenEffectScale", 1)));
                        var border = new net.minecraft.world.level.border.WorldBorder();
                        if (c.has("border") && !c.get("border").isJsonNull()) {
                            var bo = c.getAsJsonObject("border");
                            border.setCenter(bo.get("centerX").getAsDouble(), bo.get("centerZ").getAsDouble());
                            border.setSize(bo.get("size").getAsDouble());
                            border.setWarningBlocks(bo.get("warningBlocks").getAsInt());
                            border.setWarningTime(bo.get("warningTime").getAsInt());
                        }
                        put(clientLevel, ClientLevel.class, "worldBorder", border);
                        put(mc, Minecraft.class, "level", clientLevel);
                        put(actor, Entity.class, "position", new net.minecraft.world.phys.Vec3(f("camX", 0), 64, f("camZ", 0)));
                        actor.oPortalEffectIntensity = c.getAsJsonArray("portal").get(0).getAsFloat();
                        actor.portalEffectIntensity = c.getAsJsonArray("portal").get(1).getAsFloat();
                        put(hud, Hud.class, "vignetteBrightness", f("vignetteBrightness", 1));
                        put(hud, Hud.class, "scopeScale", f("scopeScale", 0.5F));
                        // The nether portal particle sprite, as the block model set would give it.
                        var contents = alloc(net.minecraft.client.renderer.texture.SpriteContents.class);
                        put(contents, net.minecraft.client.renderer.texture.SpriteContents.class, "name", Identifier.withDefaultNamespace("block/nether_portal"));
                        var sprite = alloc(net.minecraft.client.renderer.texture.TextureAtlasSprite.class);
                        put(sprite, net.minecraft.client.renderer.texture.TextureAtlasSprite.class, "contents", contents);
                        var material = new net.minecraft.client.resources.model.sprite.Material.Baked(sprite, false);
                        var model = (net.minecraft.client.renderer.block.dispatch.BlockStateModel) Proxy.newProxyInstance(Actor.class.getClassLoader(),
                            new Class<?>[] {net.minecraft.client.renderer.block.dispatch.BlockStateModel.class},
                            (proxy, method, a) -> method.getName().equals("particleMaterial") ? material : null);
                        var models = alloc(net.minecraft.client.resources.model.ModelManager.class);
                        put(models, net.minecraft.client.resources.model.ModelManager.class, "blockStateModelSet",
                            new net.minecraft.client.renderer.block.BlockStateModelSet(Map.of(), model));
                        put(mc, Minecraft.class, "modelManager", models);
                        cameraOverlays.invoke(hud, capture, delta);
                        sleep.invoke(hud, capture, delta);
                        result.addProperty("scopeScale", (float) field(Hud.class, "scopeScale").get(hud));
                    }
                    case "chat" -> {
                        for (String o : new String[] {"chatScale", "chatWidth", "chatOpacity", "chatLineSpacing", "chatHeightUnfocused", "chatHeightFocused"})
                            put(options, Options.class, o, option(c.get(o).getAsDouble()));
                        put(options, Options.class, "chatColors", option(true));
                        putStatic(Minecraft.class, "instance", mc);
                        put(gui, net.minecraft.client.gui.Gui.class, "hud", hud);
                        var listener = alloc(net.minecraft.client.multiplayer.chat.ChatListener.class);
                        put(listener, net.minecraft.client.multiplayer.chat.ChatListener.class, "delayedMessageQueue", new ArrayDeque<>());
                        put(gui, net.minecraft.client.gui.Gui.class, "chatListener", listener);
                        var C = net.minecraft.client.gui.components.ChatComponent.class;
                        put(chat, C, "minecraft", mc);
                        put(chat, C, "allMessages", new ArrayList<>());
                        put(chat, C, "trimmedMessages", new ArrayList<>());
                        put(chat, C, "visibleMessageFilter", (java.util.function.Predicate<?>) m -> true);
                        for (var e : c.getAsJsonArray("messages")) {
                            var m = e.getAsJsonObject();
                            put(hud, Hud.class, "tickCount", m.get("at").getAsInt());
                            var text = comp(m.get("text"));
                            switch (m.get("tag").isJsonNull() ? "" : m.get("tag").getAsString()) {
                                case "system" -> chat.addClientSystemMessage(text);
                                case "error" -> chat.addPlayerMessage(text, null, net.minecraft.client.multiplayer.chat.GuiMessageTag.chatError());
                                default -> chat.addPlayerMessage(text, null, null);
                            }
                        }
                        // Focused: the chat screen is open (isChatFocused), the chat is drawn in the
                        // foreground and may be scrolled.
                        boolean focused = c.has("focused") && b("focused");
                        if (focused) put(gui, net.minecraft.client.gui.Gui.class, "screen", alloc(net.minecraft.client.gui.screens.ChatScreen.class));
                        if (c.has("scroll")) chat.scrollChat(i("scroll", 0));
                        chat.extractRenderState(capture, font, i("now", 0), 0, 0, focused
                            ? net.minecraft.client.gui.components.ChatComponent.DisplayMode.FOREGROUND
                            : net.minecraft.client.gui.components.ChatComponent.DisplayMode.BACKGROUND, false);
                        put(gui, net.minecraft.client.gui.Gui.class, "screen", null);
                    }
                    default -> throw new IllegalArgumentException(c.get("kind").getAsString());
                }
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
